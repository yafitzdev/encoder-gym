// Deterministic Pi wire fixture. Uses the real CLI's reserved-turn transport;
// no production flag or application code substitutes Agent captions.
import readline from 'node:readline';
import { appendFileSync, writeFileSync } from 'node:fs';
const send = value => process.stdout.write(JSON.stringify(value) + '\n');
send({ type: 'ready', protocolVersion: 1, piPackageVersion: '0.84.4' });
let runId;
readline.createInterface({ input: process.stdin }).on('line', line => {
  const message = JSON.parse(line);
  if (message.type === 'start') {
    const request = message.request;
    runId = request.runId;
    if (request.model !== 'pinned-agent' || !request.openaiCompatible) throw Error('Wrong pinned provider');
    if (JSON.stringify(request).includes('NEVER_DISCLOSE_HOLDOUT')) throw Error('Protected evidence leaked');
    const input = JSON.parse(request.initialPrompt);
    if (request.capabilitySet === 'encoder_optimization_native_blind_v1' || request.capabilitySet === 'encoder_optimization_native_target_fit_v1') {
      appendFileSync(process.env.AGENT_FIXTURE_CALLS, JSON.stringify({model: request.model, runId, iteration: input.iteration, review: request.capabilitySet}) + '\n');
      send({ type: 'event', event: {type: 'turn_started', runId, sequence: 1} });
      const blind = request.capabilitySet === 'encoder_optimization_native_blind_v1';
      const name = blind ? 'submit_native_blind_assessments' : 'submit_native_target_fit_assessments';
      const assessments = input.rows.map(item => {
        const row = item.row ?? item;
        return blind ? {
          rowId: row.rowId,
          rowFingerprint: row.rowFingerprint,
          requestFingerprint: input.fingerprint,
          supportedCandidateIds: [row.candidates[['v3_semantic_reject', 'v3_loop_semantic_reject'].includes(process.env.AGENT_FIXTURE_LOOP) ? row.candidates.length - 1 : 0].candidateId],
          ambiguous: false,
          contextConsistent: true,
          issueCodes: [],
          rationale: 'The generated question clearly requests the first legal retrieval capability.'
        } : {
          rowId: row.rowId,
          rowFingerprint: row.rowFingerprint,
          requestFingerprint: input.fingerprint,
          blindAssessmentFingerprint: item.blindAssessmentFingerprint,
          targetFits: true,
          issueCodes: [],
          rationale: 'The generated question is a specific label-preserving variant for the target cluster.'
        };
      });
      send({type: 'tool_request', runId, callId: 'fixture-native-review', name, arguments: {assessments}});
      return;
    }
    appendFileSync(process.env.AGENT_FIXTURE_CALLS, JSON.stringify({model: request.model, runId, iteration: input.scope.iteration, evidence: input.scope.developmentEvidenceFingerprint}) + '\n');
    const turns = input.previousTurns.filter(turn => !turn.interrupted);
    const later = input.scope.iteration > 1;
    send({ type: 'event', event: {type: 'turn_started', runId, sequence: 1} });
    if (process.env.AGENT_FIXTURE_HOLD && turns.length === 1) {
      writeFileSync(process.env.AGENT_FIXTURE_HOLD, 'waiting for stop');
      return;
    }
    let name, args;
    if (input.scope.analysisProtocol === 3) {
      const twoIterationMemory = process.env.AGENT_FIXTURE_LOOP === 'v3_two_iterations';
      if (twoIterationMemory && later && !turns.length) {
        if (input.repairMemory.length !== 1) throw Error('Second V3 iteration did not receive exactly one prior outcome');
        const memory = input.repairMemory[0];
        if (memory.iteration !== 1 || memory.targetId !== 'search-variant' || memory.globalVerdict !== 'reject') throw Error('V3 outcome memory lost target or scientific rejection');
        if (memory.outputDevelopmentEvidenceFingerprint !== input.scope.developmentEvidenceFingerprint) throw Error('V3 outcome memory is not bound to the current development evidence');
        if (memory.clusterKeys.length !== 1 || memory.intervention.kind !== 'label_preserving_variants' || memory.intervention.anchorIds.length !== 1) throw Error('V3 outcome memory omitted the prior plan intervention');
      }
      const clusters = turns[0]?.tools[0]?.result?.items ?? [];
      const cluster = clusters.find(item => item.content.cluster?.dimension === 'expected_capability' && item.content.cluster?.value === 'search') ?? clusters[0];
      if (!turns.length) {
        name = 'inspect_dataset_landscape'; args = {offset: 0, limit: 20};
      } else if (turns.length === 1) {
        if (!cluster) throw Error('No V3 dataset landscape cluster');
        name = 'inspect_dataset_clusters'; args = {clusterIds: [cluster.id], cursor: 0, limit: 8};
      } else if (turns.length === 2) {
        const rows = turns[1].tools[0].result.items;
        const row = rows.find(item => item.content.question.includes('Search')) ?? rows[0];
        if (!row?.content?.investigation?.anchorFingerprint) throw Error('No V3 investigation anchor');
        name = 'preview_repair_plan';
        args = {schemaVersion: 3, summary: 'Add one precise search variant for the weak evaluated cluster.', stop: false, targets: [{
          targetId: 'search-variant', clusterKeys: [cluster.id], evidenceIds: [cluster.id],
          hypothesis: 'One context-preserving search variant may improve recall for the observed weak capability.',
          evidenceLimitations: 'Coverage is descriptive and the development failures are sampled.',
          intendedFailurePattern: 'Specific search requests rank the retrieval capability below a distractor.',
          alternativeExplanation: 'The model may need parameter changes rather than additional data.',
          operation: {kind: 'label_preserving_variants', count: {basis: 'absolute_rows', desiredRows: 1},
            allocationRationale: 'Use one inspected anchor for a minimal falsifiable change.',
            anchors: [{rowId: row.id, rowFingerprint: row.content.investigation.anchorFingerprint, additions: 1}]},
           targetMetric: {name: 'recall_at_1', direction: 'increase'}
         }]};
      } else if (twoIterationMemory && later && turns.length === 3) {
        const rejected = turns[2].tools.find(tool => tool.name === 'preview_repair_plan');
        if (!rejected?.result?.constraints?.some(constraint => constraint.code === 'repeated_unchanged_intervention')) throw Error('Repeated V3 intervention was not rejected with the typed constraint');
        const rows = turns[1].tools[0].result.items;
        const repeated = rows.find(item => item.content.question.includes('Search')) ?? rows[0];
        const row = rows.find(item => item.id !== repeated.id);
        if (!row?.content?.investigation?.anchorFingerprint) throw Error('No distinct inspected anchor for revised V3 intervention');
        name = 'preview_repair_plan';
        args = {schemaVersion: 3, summary: 'Revise the search intervention to a distinct inspected native context.', stop: false, targets: [{
          targetId: 'search-variant', clusterKeys: [cluster.id], evidenceIds: [cluster.id],
          hypothesis: 'A variant from a different inspected context may improve recall without repeating the unchanged intervention.',
          evidenceLimitations: 'The preceding candidate was rejected and its measured delta is descriptive, not causal.',
          intendedFailurePattern: 'Specific search requests rank the retrieval capability below a distractor.',
          alternativeExplanation: 'The model may need parameter changes rather than another context.',
          operation: {kind: 'label_preserving_variants', count: {basis: 'absolute_rows', desiredRows: 1},
            allocationRationale: 'Use the other inspected native context after the first intervention did not change the evidence.',
            anchors: [{rowId: row.id, rowFingerprint: row.content.investigation.anchorFingerprint, additions: 1}]},
          targetMetric: {name: 'recall_at_1', direction: 'increase'}
        }]};
      } else {
        const previewTool = turns.flatMap(turn => turn.tools).find(tool => tool.name === 'preview_repair_plan' && tool.result?.preview?.fingerprint);
        if (!previewTool?.result?.preview?.fingerprint) throw Error('No accepted V3 preview');
        name = 'submit_repair_plan';
        args = {plan: previewTool.arguments, previewFingerprint: previewTool.result.preview.fingerprint};
      }
    } else if (input.scope.analysisProtocol === 2) {
      const clusters = turns[0]?.tools[0]?.result?.items ?? [];
      const cluster = clusters.find(item => item.content.cluster?.dimension === 'expected_capability' && item.content.cluster?.value === 'search') ?? clusters[0];
      if (!turns.length) {
        name = 'inspect_dataset_landscape'; args = {offset: 0, limit: 20};
      } else if (turns.length === 1) {
        if (!cluster || cluster.content.kind !== 'dataset_cluster') throw Error('No dataset landscape cluster');
        if (later) {
          const examples = cluster.content.development?.flatMap(value => value.sampledFailureDiagnostics?.examples ?? []) ?? [];
          if (!examples.some(example => example.expectedRank === 3 && example.questionPreview.includes('candidate regression'))) throw Error('Second iteration did not receive candidate evaluation');
        }
        name = 'inspect_dataset_clusters'; args = {clusterIds: [cluster.id], examplesPerCluster: 4};
      } else {
        const rows = turns[1].tools[0].result.items;
        if (!cluster || !rows.length) throw Error('No real cluster evidence or inspected row');
        const row = rows.find(item => item.content.question.includes(later ? 'Retain' : 'Search')) ?? rows[0];
        if (process.env.AGENT_FIXTURE_LOOP === 'no_change_first' || (later && (process.env.AGENT_FIXTURE_LOOP === 'no_change' || input.scope.iteration === 3))) {
          name = 'propose_dataset_edits'; args = {summary: 'The candidate regression does not justify another dataset change.', stop: true, removals: [], additions: []};
        } else {
          name = 'propose_dataset_edits';
          args = {summary: 'Shift search coverage by one row to address the weak search cluster.', stop: false,
            removals: [{rowId: row.id, reason: 'Ambiguous wording conflicts with the weak inspected cluster.', evidenceIds: [cluster.id]}],
            additions: [{templateRowId: row.id, instruction: 'Add one specific search request for the weak search cluster.', count: 1, evidenceIds: [cluster.id]}]};
          if (later) {
            if (input.scope.maximumRowChanges !== 6) throw Error('Cumulative edit budget was reset');
            args = {summary: 'The candidate regression points to conflicting search coverage; remove one inspected row.', stop: false,
              removals: [{rowId: row.id, reason: 'Changed candidate evidence identifies conflicting coverage.', evidenceIds: [cluster.id]}], additions: []};
          }
        }
      }
    } else if (!turns.length) {
      name = 'inspect_development_failures'; args = {offset: 0, limit: 20};
    } else if (turns.length === 1) {
      if (later) {
        const evidence = turns[0].tools[0].result.items;
        const failure = evidence.find(item => item.content.evidenceScope === 'retrieval_failure_sample');
        if (failure?.content.expectedRank !== 3 || !failure.content.question.includes('candidate regression')) throw Error('Second iteration did not receive candidate evaluation');
        if (!evidence.some(item => item.content.kind === 'development_summary' && item.content.assessments)) throw Error('Previous metrics and decision missing');
      }
      if (process.env.AGENT_FIXTURE_LOOP === 'no_change_first' || (later && (process.env.AGENT_FIXTURE_LOOP === 'no_change' || input.scope.iteration === 3))) {
        name = 'propose_dataset_edits'; args = {summary: 'The candidate regression does not justify another dataset change.', stop: true, removals: [], additions: []};
      } else {
        name = 'inspect_training_rows'; args = {offset: 0, limit: 20, query: later ? 'Retain' : 'Search'};
      }
    } else {
      const evidence = turns[0].tools[0].result.items.find(item => item.content.evidenceScope === 'retrieval_failure_sample');
      if (!turns[0].tools[0].result.items.some(item => item.content.kind === 'development_summary')) throw Error('Metrics are not on the first inspection page');
      const rows = turns[1].tools[0].result.items;
      if (!evidence || !rows.length) throw Error('No real inspected evidence');
      if (evidence.content.evidenceScope !== 'retrieval_failure_sample' || evidence.content.sampleLimit !== 50) throw Error('Diagnostic sample was not labeled');
      name = 'propose_dataset_edits';
      args = {summary: 'Replace the ambiguous search example and add the missing specific search request.', stop: false,
        removals: [{rowId: rows[0].id, reason: 'Ambiguous wording conflicts with the inspected retrieval failure.', evidenceIds: [evidence.id]}],
        additions: [{templateRowId: rows[0].id, instruction: 'Cover the specific search request from the failure.', count: 1, evidenceIds: [evidence.id]}]};
      if (later) {
        if (input.scope.maximumRowChanges !== 6) throw Error('Cumulative edit budget was reset');
        args = {summary: 'The candidate regression points to conflicting retain coverage; remove that inspected row.', stop: false,
          removals: [{rowId: rows[0].id, reason: 'Changed candidate evidence identifies conflicting retain coverage.', evidenceIds: [evidence.id]}], additions: []};
      }
    }
    if (process.env.AGENT_FIXTURE_LOOP === 'canary_rejected' && name === 'propose_dataset_edits') args.additions[0].count = 17;
    send({type: 'tool_request', runId, callId: 'fixture-tool', name, arguments: args});
  } else if (message.type === 'tool_result') {
    send({type: 'event', event: {type: 'turn_completed', runId, sequence: 1, inputTokens: 150, outputTokens: 70, costMicrousd: 0}});
    send({type: 'completed', runId});
  } else if (message.type === 'tool_error') {
    send({type: 'failed', runId, message: message.message});
  } else if (message.type === 'cancel') process.exit(0);
});
