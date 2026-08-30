use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, bail};
use generation_core::{
    dimensions::expand_generation_cells,
    ports::{DatasetStore, RowStore},
};
use synthetic_data_sqlite::SqliteStore;
use workflow_core::{
    allocation::{
        ExplicitCellTarget, InitialAllocationFeasibility, InitialAllocationPolicy,
        InitialAllocationRecord, InitialAllocationRequest, InitialCellConstraint,
        InitialCellCoverage, allocate_initial_budget,
    },
    ports::{InitialAllocationQuery, InitialAllocationStore},
};

use crate::cli::{AllocationCommand, InitialAllocationArgs, InitialAllocationPolicyArg};
use crate::document::read as read_document;

pub async fn execute(command: AllocationCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        AllocationCommand::Preview(args) => {
            let result = calculate(args, store).await?;
            crate::presentation::print(&result)
        }
        AllocationCommand::Create(args) => {
            let dataset_id = args.dataset_id;
            let result = calculate(args, store).await?;
            if result.feasibility != InitialAllocationFeasibility::Feasible {
                bail!(
                    "initial allocation is infeasible: {}",
                    serde_json::to_string(&result.issues)?
                );
            }
            let dataset = store
                .get_dataset(dataset_id)
                .await?
                .with_context(|| format!("dataset not found: {dataset_id}"))?;
            let plan = result.to_generation_plan(&dataset)?;
            let record = InitialAllocationRecord::new(result, plan.id)?;
            store.create_initial_allocation(&record, &plan).await?;
            crate::presentation::print(&record)
        }
        AllocationCommand::Show { id } => {
            let allocation = store
                .get_initial_allocation(id)
                .await?
                .with_context(|| format!("initial allocation not found: {id}"))?;
            crate::presentation::print(&allocation)
        }
        AllocationCommand::List { dataset_id, page } => {
            let allocations = store
                .query_initial_allocations(InitialAllocationQuery {
                    dataset_id,
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&allocations, allocations.len(), page)
        }
    }
}

async fn calculate(
    args: InitialAllocationArgs,
    store: &SqliteStore,
) -> anyhow::Result<workflow_core::allocation::InitialAllocationResult> {
    let dataset = store
        .get_dataset(args.dataset_id)
        .await?
        .with_context(|| format!("dataset not found: {}", args.dataset_id))?;
    let counts = store.dataset_cell_counts(dataset.id).await?;
    let cells = expand_generation_cells(&dataset)
        .into_iter()
        .map(|cell| (cell.key(), cell))
        .collect::<BTreeMap<_, _>>();
    let current_coverage = counts
        .into_iter()
        .map(|(key, counts)| {
            let cell = cells
                .get(&key)
                .cloned()
                .with_context(|| format!("persisted coverage contains unknown cell: {key}"))?;
            Ok(InitialCellCoverage {
                cell,
                accepted: counts.accepted,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let policy = load_policy(&args)?;
    let constraints = args
        .constraints
        .as_deref()
        .map(read_constraints)
        .transpose()?
        .unwrap_or_default();
    allocate_initial_budget(
        &dataset,
        InitialAllocationRequest {
            total_rows: args.total_rows,
            reserved_rows: args.reserved_rows,
            policy,
            current_coverage,
            constraints,
        },
    )
    .map_err(Into::into)
}

fn load_policy(args: &InitialAllocationArgs) -> anyhow::Result<InitialAllocationPolicy> {
    match args.policy {
        InitialAllocationPolicyArg::Balanced => {
            reject_unused(args, false, false, false)?;
            Ok(InitialAllocationPolicy::Balanced)
        }
        InitialAllocationPolicyArg::Weighted => {
            reject_unused(args, true, false, false)?;
            Ok(InitialAllocationPolicy::Weighted {
                weights: args
                    .weights
                    .as_deref()
                    .map(read_document)
                    .transpose()?
                    .unwrap_or_default(),
            })
        }
        InitialAllocationPolicyArg::MinimumThenWeighted => {
            reject_unused(args, true, true, false)?;
            let minimum_per_cell = args
                .minimum_per_cell
                .context("--minimum-per-cell is required for minimum-then-weighted")?;
            Ok(InitialAllocationPolicy::MinimumThenWeighted {
                minimum_per_cell,
                weights: args
                    .weights
                    .as_deref()
                    .map(read_document)
                    .transpose()?
                    .unwrap_or_default(),
            })
        }
        InitialAllocationPolicyArg::Explicit => {
            reject_unused(args, false, false, true)?;
            let path = args
                .targets
                .as_deref()
                .context("--targets is required for explicit")?;
            Ok(InitialAllocationPolicy::Explicit {
                targets: read_targets(path)?,
            })
        }
    }
}

fn reject_unused(
    args: &InitialAllocationArgs,
    allow_weights: bool,
    allow_minimum: bool,
    allow_targets: bool,
) -> anyhow::Result<()> {
    if args.weights.is_some() && !allow_weights {
        bail!("--weights is not valid for the selected allocation policy");
    }
    if args.minimum_per_cell.is_some() && !allow_minimum {
        bail!("--minimum-per-cell is not valid for the selected allocation policy");
    }
    if args.targets.is_some() && !allow_targets {
        bail!("--targets is not valid for the selected allocation policy");
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum TargetsDocument {
    List(Vec<ExplicitCellTarget>),
    Wrapped { targets: Vec<ExplicitCellTarget> },
}

fn read_targets(path: &Path) -> anyhow::Result<Vec<ExplicitCellTarget>> {
    Ok(match read_document(path)? {
        TargetsDocument::List(targets) | TargetsDocument::Wrapped { targets } => targets,
    })
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum ConstraintsDocument {
    List(Vec<InitialCellConstraint>),
    Wrapped {
        constraints: Vec<InitialCellConstraint>,
    },
}

fn read_constraints(path: &Path) -> anyhow::Result<Vec<InitialCellConstraint>> {
    Ok(match read_document(path)? {
        ConstraintsDocument::List(constraints) | ConstraintsDocument::Wrapped { constraints } => {
            constraints
        }
    })
}
