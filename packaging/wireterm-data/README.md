# WireTerm adjacent data

Keep this directory beside `wireterm.exe`.

- `default-playlist.json` is applied once, as immutable revision 1, only when
  no Playlist revision exists. It points to the bundled direct image folder at
  `images/default-playlist`; existing Playlist revisions are never replaced.
- `playlist-revisions` contains immutable Playlist edits after initialization.
- Extension secret inputs are masked in the editor but stored without
  encryption inside Playlist revisions; protect access to this folder.
- `extensions` contains discoverable Extension folders. The shipped
  `http-extension` is editable and may be removed.

WireTerm owns no data outside the extracted portable folder.
