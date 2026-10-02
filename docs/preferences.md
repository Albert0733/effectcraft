# Settings

## Settings (Preferences)

EffectCraft ▸ Settings (macOS) or Edit ▸ Preferences (Windows/Linux), `Cmd+Alt+;`. The dialog has
After Effects 2026's pages (General, Startup & Repair, Project, Composition, Previews, Appearance,
Grids & Guides, Labels, Type, Import, Export, Audio, Disk, Memory & CPU, Video, 3D, Scripting &
Expressions) with OK, Cancel, Previous and Next. Older page names still open the right page
(`prefs.open {"page": "Auto-Save"}` opens Project, `Media & Disk Cache` opens Disk, `Memory &
Performance` opens Memory & CPU).

The model is `effectcraft_engine::prefs::Prefs`: serde, versioned (`version`), addressed by dotted
keys (`general.undoLevels`, `autoSave.intervalMinutes`, `labels.3.name`). It is stored as
`prefs.json` through the session's `ConfigStore` (the desktop app uses the platform config
directory: `~/Library/Application Support/EffectCraft` on macOS, `%APPDATA%\EffectCraft` on
Windows, `$XDG_CONFIG_HOME/effectcraft` on Linux; the web app can back the same trait with
`localStorage`). Loading migrates older layouts, keeps unknown keys (from newer versions) and
falls back to defaults for values that don't parse.

Commands (CLI, MCP, control channel):

| Command | Parameters |
|---|---|
| `prefs.get` | `{key?}` (omit for everything) |
| `prefs.set` | `{key, value}` or `{values: {key: value}}` |
| `prefs.reset` | `{page?}` (omit for everything) |
| `prefs.open` | `{page?}` opens the Settings dialog |
| `prefs.pages` | the page schema: every row, its key, type, range and whether it is wired |

### Settings that change behaviour

- `general.undoLevels`: Levels of Undo
- `general.pathPointSize`: Path Point and Handle Size
- `general.recentItems`: Recent Projects Shown
- `general.showToolTips`: Show Tool Tips
- `general.createLayersAtCompStart`: Create Layers at Composition Start Time
- `general.defaultSpatialLinear`: Default Spatial Interpolation to Linear
- `startup.showHomeOnLaunch`: Show Home Screen When Launching
- `startup.offerCrashRecovery`: Offer to Open the Latest Auto-Save After a Crash
- `project.useTemplate`: New Project Loads Template
- `project.templatePath`: Template Project
- `autoSave.enabled`: Automatically Save Projects
- `autoSave.intervalMinutes`: Save Every
- `autoSave.maxVersions`: Maximum Project Versions
- `autoSave.location`: Auto-Save Location
- `autoSave.folder`: Custom Location
- `autoSave.saveOnRenderStart`: Save When Starting Render Queue
- `composition.showRenderingProgress`: Show Rendering Progress in Info Panel and Flowchart
- `previews.adaptiveResolutionLimit`: Adaptive Resolution Limit
- `previews.cacheFramesWhenIdle`: Cache Frames When Idle
- `previews.fastPreviews`: Fast Previews (Draft 3D, Faster Effects)
- `appearance.theme`: Theme
- `appearance.brightness`: Brightness
- `appearance.useLabelColorForHandles`: Use Label Color for Layer Handles and Paths
- `grids.gridColor`: Color
- `grids.gridSpacing`: Gridline Every
- `grids.gridSubdivisions`: Subdivisions
- `grids.guideColor`: Color
- `grids.actionSafe`: Action-safe
- `grids.titleSafe`: Title-safe
- `import.stillFootage`: Still Footage
- `import.stillSeconds`: Still Duration
- `import.sequenceFps`: Sequence Footage
- `audio.outputDevice`: Default Output (device list from the desktop host)
- `audio.outputLeft`: Left (Audio Output Mapping)
- `audio.outputRight`: Right (Audio Output Mapping)
- `memory.layerCacheMb`: Layer Cache
- `memory.mediaCacheMb`: Footage Frame Cache
- `memory.previewCacheMb`: Preview (RAM) Cache
- `threeD.defaultRenderer`: Default 3D Renderer
- `labels.N.name` / `labels.N.color`: the 16 label names and colours, used by the Label menu,
  the timeline, project panel, render queue and viewer handles

### Not wired yet (TODO)

These rows are in the dialog for After Effects parity and are stored, but nothing reads them yet.
`prefs.pages` reports them with `"live": false`. A test keeps this list in step with the schema.

- `general.switchesAffectNestedComps`: Switches Affect Nested Comps
- `general.preserveConstantVertexCount`: Preserve Constant Vertex and Feather Point Count when Editing Masks
- `general.syncTimeRelatedItems`: Synchronize Time of All Related Items
- `general.expressionPickWhipCompact`: Expression Pick Whip Writes Compact English
- `general.createSplitLayersAbove`: Create Split Layers Above Original Layer
- `general.useSystemColorPicker`: Use System Color Picker
- `startup.showHomeOnOpenProject`: Show Home Screen When Opening a Project
- `composition.motionPath`: Motion Path
- `composition.motionPathSeconds`: Seconds
- `composition.motionPathKeyframes`: Keyframes
- `composition.hardwareAcceleratePanels`: Hardware Accelerate Composition, Layer and Footage Panels
- `previews.showInternalWireframes`: Show Internal Wireframes
- `previews.zoomQuality`: Viewer Zoom Quality
- `appearance.useLabelColorForTabs`: Use Label Color for Related Tabs
- `appearance.cycleMaskColors`: Cycle Mask Colors
- `appearance.useGradients`: Use Gradients
- `grids.gridStyle`: Style
- `grids.proportionalHorizontal`: Horizontal
- `grids.proportionalVertical`: Vertical
- `grids.guideStyle`: Style
- `type.textEngine`: Text Engine
- `type.fontPreview`: Show Font Preview
- `type.recentFonts`: Number of Recent Fonts to Display
- `type.fontNamesInEnglish`: Show Font Names in English
- `import.reportMissingFrames`: Report Missing Frames
- `import.unlabeledAlpha`: Interpret Unlabeled Alpha As
- `import.dragImportAs`: Default Drag Import As
- `export.defaultOutputFolder`: Default Output Folder
- `export.segmentSequences`: Segment Sequences
- `export.segmentSequenceFiles`: Files per Segment
- `export.segmentMovies`: Segment Movie Files
- `export.segmentMovieMb`: Segment Size
- `export.appendBitsToName`: Append Bit Depth to File Name
- `audio.previewSampleRate`: Preview Sample Rate
- `disk.diskCacheEnabled`: Enable Disk Cache
- `disk.diskCacheMaxGb`: Maximum Disk Cache Size
- `disk.diskCacheFolder`: Disk Cache Folder
- `disk.mediaCacheFolder`: Database and Cache Folder
- `disk.conformedMediaFolder`: Conformed Audio Folder
- `memory.ramReservedGb`: RAM Reserved for Other Applications
- `memory.reduceCacheWhenLow`: Reduce Cache Size When System Is Low on Memory
- `video.enableOutput`: Enable Video Preview Output
- `video.device`: Video Device
- `video.outputDuringPlayback`: Video Output During Playback
- `video.mirrorOnMonitor`: Mirror on Computer Monitor
- `video.disableWhenBackground`: Disable Video Output When in Background
- `threeD.showReferenceAxes`: Show 3D Reference Axes
- `threeD.extendedViewer`: Extended Viewer
- `threeD.realtimeShadows`: Realtime Shadows in Draft
- `scripting.allowScriptsWriteFiles`: Allow Scripts to Write Files and Access Network
- `scripting.warnExecutingFiles`: Warn User When Executing Files
- `scripting.enableJsDebugger`: Enable JavaScript Debugger
- `scripting.editorFontSize`: Font Size
- `scripting.syntaxHighlighting`: Syntax Highlighting
- `scripting.lineNumbers`: Line Numbers
- `scripting.autoComplete`: Auto-complete
- `scripting.bracketMatching`: Bracket Matching
- `scripting.wordWrap`: Word Wrap
- `scripting.errorBanner`: Show Expression Error Banner
