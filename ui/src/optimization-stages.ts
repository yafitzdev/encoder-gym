export const optimizationStages = ["checking_inputs", "preparing_data", "starting", "training", "saving_candidate", "evaluating"] as const;
export type OptimizationStage = (typeof optimizationStages)[number];
export const stageLabels: Record<OptimizationStage, string> = {
  checking_inputs: "Checking inputs", preparing_data: "Preparing data", starting: "Starting",
  training: "Training", saving_candidate: "Saving candidate", evaluating: "Evaluating",
};
export function isOptimizationStage(value: unknown): value is OptimizationStage {
  return optimizationStages.includes(value as OptimizationStage);
}
/** Native execution includes training, checkpoint saving and development evaluation. */
export function taskStage(task: string): OptimizationStage | undefined {
  if (["agent_analysis", "data_generation"].includes(task)) return "preparing_data";
  if (["evaluating_retrieval", "evaluating_agent", "development_decision", "final_decision", "finalizing_iteration"].includes(task)) return "evaluating";
  if (["saving_checkpoint", "registering_candidate"].includes(task)) return "saving_candidate";
  if (["checking_files", "checking_training_data", "loading_model", "preparing_batches", "training"].includes(task)) return "training";
  if (["loading_training_rows", "writing_training_rows", "checking_materialized_project"].includes(task)) return "preparing_data";
  if (["loading_evaluation_protocol", "creating_candidate", "creating_experiment"].includes(task)) return "starting";
  if (["checking_model", "checking_dataset", "checking_runtime"].includes(task)) return "checking_inputs";
  return undefined;
}
export const operationStages: Record<string, OptimizationStage> = {
  "optimization.agent": "preparing_data", "optimization.generation": "preparing_data",
  "optimization.qualification": "preparing_data",
  "optimization.start": "checking_inputs", "optimization.prepare": "checking_inputs",
  "optimization.materialize": "preparing_data", "optimization.attach_experiment": "starting",
  "optimization.execute": "training", "optimization.register_candidate": "saving_candidate",
  "optimization.final_evaluation": "evaluating",
};
