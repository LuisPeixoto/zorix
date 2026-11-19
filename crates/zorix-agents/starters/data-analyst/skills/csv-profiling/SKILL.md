---
name: csv-profiling
description: "Profiles a CSV file: columns, types, missing values and basic statistics. Use at the start of any question about a CSV file, before computing anything on it."
license: MIT
compatibility: "Runs in the ZORIX sandbox with Python's standard library only."
metadata:
  author: zorix
  version: "1"
---

# CSV profiling

Profile the file first, so you know what it holds. There are two ways to run `scripts/profile.py`:

- **Text you have** (pasted by the person, or at most 100 KB): `skill.run` with the CSV text as `stdin`.

  ```
  skill.run  name: csv-profiling  script: scripts/profile.py  stdin: "<the CSV text>"
  ```

- **A file in the workspace**: read the script with `skill.read` (`scripts/profile.py`) and run it with `code.run_in_workspace`, passing the file's path as its first argument. Skill scripts cannot see the workspace themselves.

The script prints one JSON object with:

- `rows`: how many data rows the file has;
- `columns`: for each column its name, the type it seems to have (`integer`, `number`, `date` or `text`), how many values are missing and how many differ;
- for number columns: `min`, `max`, `mean` and `median`;
- for text columns: the five most common values.

Use that to decide what to compute. Then write your own code for the actual question.

The script reads at most 200000 rows and says when it stopped early.
