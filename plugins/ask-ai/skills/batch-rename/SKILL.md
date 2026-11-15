---
name: batch-rename
description: Rename several local files with a shared rule, such as removing a common prefix, moving numbers from the end to the start, adding sequential numbers, or revising an earlier batch rename proposal. Use when the user asks to perform these changes.
---

# Batch rename

1. If the user has not attached files or a directory, ask them to select items in Explorer and open Ask AI with Alt+X. An attached directory exposes only its immediate regular files. If the requested scope is unclear, ask which files to include.
2. Call `files_list` to get the file IDs and current names. Use the listed order for sequential numbering unless the user requests a different order. For a revision, use `previous_proposal` when it is present.
3. Compute each complete new filename. Change only what the user requested. Preserve extensions, numbering width and capitalization unless the request changes them. Skip unchanged names, and ask for clarification if the rule cannot be determined from the names.
4. Call `files_propose_change` with one complete proposal. Use the listed IDs and basenames, including extensions. For example:

```json
{
  "operation": "rename",
  "changes": [
    {"file": 0, "name": "01-Home.png"},
    {"file": 1, "name": "02-Settings.png"}
  ]
}
```

5. If validation reports a conflict, resolve it according to the user's instructions or ask for clarification. The tool renders the verified before/after table and directory scope with a confirmation button. The button submits the plugin's confirmation action for that exact plan. A new instruction replaces the pending plan and requires fresh confirmation.
