# FilmCraft app icon

The FilmCraft app icon is an original geometric **F monogram** on the app's colour
field. The letter is paired with a small visual cue for the app's work: image editing, vector
paths, documents, light, film or effects. Its flat, high-contrast design is made to remain legible
at small sizes. It is an independent mark and does not reproduce Adobe or ArtCraft brand marks.

## Artwork and licence

The icon is an original monogram created for the Craft App Suite by OpenAI Codex at the ArtCraft
Team's request. The source SVGs are licensed under MIT OR Apache-2.0; see [`LICENSE.txt`](LICENSE.txt).

## Files

- `filmcraft.svg`: canonical 512-unit SVG master.
- `filmcraft-small.svg`: lightweight copy used for scalable Linux and web display.
- `filmcraft-1024.png`, `filmcraft.icns`, and `filmcraft.ico`: generated platform artwork for macOS and Windows.
- `hicolor/`: generated Linux icons.
- The app embeds the generated icon in its installer and runtime window/Dock icon.

## Regenerate

Run `packaging/icons.sh` from the repository root. It uses `resvg` and, on macOS, `iconutil`.
