# LensMap JetBrains Plugin

JetBrains integration for LensMap with a persistent note browser tool window.

## Features

- `LensMap` tool window with a structured note list and detail pane
- `LensMap > Show Current File Notes`
- `LensMap > Search Workspace Notes`
- `LensMap > Add Note at Caret`
- `LensMap > Edit Note at Caret`
- Open the selected note in source, open the backing LensMap file, copy its ref or note text, and edit the selected entry from the tool window
- English and Chinese prompts/notifications

## Build

```bash
cd editor/jetbrains
./gradlew buildPlugin
```

The packaged plugin ZIP is written to `build/distributions/`.
