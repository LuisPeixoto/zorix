---
name: review-checklist
description: "A checklist for reviewing code changes, ordered by how much a problem costs. Use when reviewing a diff, a pull request or a file for correctness and security."
license: MIT
metadata:
  author: zorix
  version: "1"
---

# Review checklist

Go from the most expensive problems to the cheapest. Stop at the first level that has findings only if the rest would be noise.

1. **Correctness**: wrong conditions, off-by-one errors, unhandled empty or missing values, errors that are swallowed, resources that are never released, concurrent access without protection.
2. **Security**: input that reaches a query, a command, a path or a template without escaping; secrets in code or logs; missing checks on who may do something; unsafe deserialisation.
3. **Data**: changes that cannot be undone, migrations that lock a table, loss of precision with money or time.
4. **Tests**: does a test fail without the change? Is the unhappy path covered?
5. **Clarity**: names that lie, functions that do two things, comments that say what instead of why.

`references/security.md` lists the patterns worth a second look. Report each finding with the file, the line, what is wrong and a fix.
