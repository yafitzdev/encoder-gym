/** Presentation only: preserve the original diagnostic; never decide backend policy. */
export function describeFailure(error: unknown): { title: string; recovery: string; detail: string } {
  const detail = error instanceof Error ? error.message : String(error);
  const known: [RegExp, string, string][] = [
    [/project collection/i, "The project library couldn't be read", "Check the technical details for its location and cause, then try again. The saved library has not been overwritten."],
    [/project name between|Invalid project name/i, "Choose a valid project name", "Use 1–120 characters, without control characters or a blank name."],
    [/Invalid dataset name/i, "Choose a valid dataset name", "Use 1–120 characters, without control characters or a blank name."],
    [/single new folder name/i, "Enter a folder name, not a path", "Use Choose location for the parent folder. Enter only the new folder's name in the name field."],
    [/not an Encoder Gym workspace/i, "This folder is not a Gym project", "Open a folder containing encoder-gym.json. To start from a checkpoint, use New project."],
    [/different project identity|different Gym project/i, "This is a different project", "Use Locate folder to select this project's moved folder. Use Open project to add a different project."],
    [/non-training partition/i, "This file contains held-out data", "Choose its actual purpose before selecting it again, or choose a training-only file. Held-out rows cannot be imported as training data."],
    [/os error 2|ENOENT|no longer available|folder no longer exists|folder does not exist/i, "A required file or folder is missing", "Check that the drive and folder are available. If the project moved, use Locate folder."],
    [/permission denied|access is denied|EACCES/i, "This location cannot be accessed", "Check the folder's permissions and whether another application has locked the file, then try again."],
    [/checksum|fingerprint.*(mismatch|changed)|content.*changed|changed after preview/i, "The files no longer match", "Review the technical details. Re-select changed source files for a new preview; do not overwrite existing project artifacts."],
    [/destination.*(exists|exist)|already exists/i, "That destination already exists", "Choose a new folder name for creation, or use Open project for an existing Gym workspace."],
  ];
  for (const [pattern, title, recovery] of known) if (pattern.test(detail)) return { title, recovery, detail };
  return { title: "The operation couldn't finish", recovery: "Review the technical details, check your selection, and try again.", detail };
}

/** Remove desktop transport wrappers while preserving the backend's actual reason. */
export function failureReason(error: unknown): string {
  let reason = error instanceof Error ? error.message : String(error);
  reason = reason.replace(/^Error invoking remote method '[^']+':\s*/i, "");
  while (/^Error:\s*/i.test(reason)) reason = reason.replace(/^Error:\s*/i, "");
  return reason.trim() || "Unknown error.";
}
