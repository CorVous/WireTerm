---
status: accepted
---

# Store secrets as Extension inputs

An Extension declares `secret` inputs alongside its other settings, and each Extension Playlist Item stores its own values in its Playlist revision. The editor masks these fields but WireTerm does not encrypt them; this removes the separate named-secret library and binding workflow and lets one self-describing Lua script receive and use every field it declares. Portable data must therefore be treated as sensitive.
