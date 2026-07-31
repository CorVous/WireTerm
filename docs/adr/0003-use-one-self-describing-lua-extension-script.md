---
status: accepted
---

# Use one self-describing Lua Extension script

Each Extension is one self-describing Lua script plus relative local assets. The script exposes metadata, its user-input schema, and a render entry point that may use a narrow WireTerm host API for bounded HTTP, named-secret references, clock, and assets before returning fixed 800 × 480 SVG; this replaces manifest-defined requests, Liquid templates, and separate transforms while preserving the pure-Rust SVG-to-panel boundary.
