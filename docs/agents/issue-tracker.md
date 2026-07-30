# Issue tracker: GitHub

Issues and product requirements for this repository live as GitHub issues. Use the GitHub CLI for issue operations.

## Conventions

- Create issues with `gh issue create`.
- Read issues and comments with `gh issue view`.
- List, comment on, label, edit, and close issues with the corresponding `gh issue` commands.

The repository is inferred from the Git remote when commands run inside this clone.

## Pull requests as a triage surface

**PRs as a request surface: no.**

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Read the relevant GitHub issue and its comments.

## Wayfinding operations

Wayfinder uses one GitHub issue as the map and linked child issues as tickets. Map issues use the `wayfinder:map` label. Child tickets use `wayfinder:research`, `wayfinder:prototype`, `wayfinder:grilling`, or `wayfinder:task` as appropriate.

Use GitHub sub-issues for child tickets when available. If sub-issues are unavailable, link tickets from the map's task list and state their map relationship in the ticket body. Represent blockers with GitHub issue dependencies when available; otherwise, record blocking issue references in the ticket body. A ticket may be claimed by assigning it to the driving developer, and it is resolved by documenting the result, closing it, and recording the resulting decision in the map.
