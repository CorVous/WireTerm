# Domain Docs

How engineering skills should consume WireTerm's domain documentation when exploring the codebase.

## Before exploring, read these

- `CONTEXT.md` at the repository root, when it exists.
- Relevant architecture decision records in `docs/adr/`, when they exist.

If these files do not exist, proceed silently. The domain-modeling workflow creates them when terminology or decisions are actually resolved.

## File structure

WireTerm is a single-context repository:

```
/
├── CONTEXT.md
├── docs/adr/
└── src/
```

## Use the glossary's vocabulary

When naming a domain concept, use the term defined in `CONTEXT.md`. If a needed concept is absent, reconsider whether an existing term fits or record the gap for domain modeling.

## Flag ADR conflicts

If proposed work contradicts an existing ADR, identify that conflict explicitly instead of silently overriding it.
