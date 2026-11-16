# Figma Plugin API Reference

> Part of the [use_figma skill](../SKILL.md). What works and what doesn't in the `use_figma` environment.

## Contents

- Node Creation
- Grouping and Boolean Operations
- Library Imports
- Variables API
- Core Properties
- Node Manipulation
- Descriptions and Documentation Links
- SVG and Images
- Utilities and Plugin Lifecycle
- Node Traversal
- Unsupported APIs


## Node Creation (Design Mode)

```js
figma.createRectangle()
figma.createFrame()
figma.createComponent()         // Creates a ComponentNode
figma.createText()
figma.createEllipse()
figma.createStar()
figma.createLine()
figma.createVector()
figma.createPolygon()
figma.createBooleanOperation()
figma.createSlice()
figma.createPage()              // Page node can be created, but child persistence is limited in headless mode
figma.createSection()
figma.createTextPath()
```

## Grouping & Boolean Operations

```js
figma.group(nodes, parent, index?)              // Group nodes
figma.flatten(nodes, parent?, index?)           // Flatten to vector
figma.union(nodes, parent?, index?)             // Boolean union
figma.subtract(nodes, parent?, index?)          // Boolean subtract
figma.intersect(nodes, parent?, index?)         // Boolean intersect
figma.exclude(nodes, parent?, index?)           // Boolean exclude
figma.combineAsVariants(components, parent?)    // Combine ComponentNodes into ComponentSet (Design/Sites only)
```

## Library Component Import

These methods import components from **team libraries** (not the same file you're working in). For components in the current file, use `use_figma` with `figma.getNodeByIdAsync()` or `findOne()`/`findAll()` to locate them directly.

```js
// Import a published component from a team library by key
const comp = await figma.importComponentByKeyAsync("COMPONENT_KEY")
const instance = comp.createInstance()

// Import a published component set from a team library by key
const compSet = await figma.importComponentSetByKeyAsync("COMPONENT_SET_KEY")
const variant =
  compSet.children.find((c) => c.type === "COMPONENT" && c.name.includes("size=md")) ||
  compSet.defaultVariant
const variantInstance = variant.createInstance()
```

## Library Style Import (Team Libraries)

These methods import styles from **team libraries** (not the same file). For styles in the current file, use `figma.getLocalPaintStyles()`, `figma.getLocalTextStyles()`, etc.

```js
// Import a published style from a team library by key
const style = await figma.importStyleByKeyAsync("STYLE_KEY")

// Apply the imported style to a node
await node.setFillStyleIdAsync(style.id)    // for PaintStyle as fill
await node.setStrokeStyleIdAsync(style.id)  // for PaintStyle as stroke
await node.setTextStyleIdAsync(style.id)    // for TextStyle
await node.setEffectStyleIdAsync(style.id)  // for EffectStyle
await node.setGridStyleIdAsync(style.id)    // for GridStyle
```

## Library Variable Import (Team Libraries)

This imports variables from **team libraries** (not the same file). For variables in the current file, use `figma.variables.getLocalVariablesAsync()` or `figma.variables.getVariableByIdAsync()`.

```js
// Import a published variable from a team library by key
const variable = await figma.variables.importVariableByKeyAsync("VARIABLE_KEY")

// Bind the imported variable to node properties
node.setBoundVariable("width", variable)           // FLOAT variable

// Bind to fills/strokes (COLOR variable) — returns a NEW paint, must capture it
const newPaint = figma.variables.setBoundVariableForPaint(paintCopy, "color", variable)
node.fills = [newPaint]
```

## Variables API

```js
// Collections
const collection = figma.variables.createVariableCollection("Name")
collection.name                           // Get/set name
collection.modes                          // Array of {modeId, name} — starts with 1 mode
collection.addMode("Dark")               // Returns new modeId string
collection.renameMode(modeId, "Light")

// Variables
const variable = figma.variables.createVariable("name", collection, "COLOR")
//                                                       ^ must be a collection object (passing an ID string is deprecated)
// resolvedType: "COLOR" | "FLOAT" | "STRING" | "BOOLEAN"
variable.setValueForMode(modeId, value)

// Scopes — controls where variable appears in property pickers
variable.scopes = ["FRAME_FILL", "SHAPE_FILL"]   // only fill pickers
variable.scopes = ["TEXT_FILL"]                    // only text color picker
variable.scopes = ["STROKE_COLOR"]                 // only stroke picker
variable.scopes = []                               // hidden from all pickers (use for primitives)
// All valid scope values:
//   ALL_SCOPES, TEXT_CONTENT, CORNER_RADIUS, WIDTH_HEIGHT, GAP,
//   ALL_FILLS, FRAME_FILL, SHAPE_FILL, TEXT_FILL,
//   STROKE_COLOR, STROKE_FLOAT, EFFECT_FLOAT, EFFECT_COLOR,
//   OPACITY, FONT_FAMILY, FONT_STYLE, FONT_WEIGHT, FONT_SIZE,
//   LINE_HEIGHT, LETTER_SPACING, PARAGRAPH_SPACING, PARAGRAPH_INDENT

// Querying (always use the Async variants — sync versions are deprecated)
await figma.variables.getVariableByIdAsync(id)
await figma.variables.getLocalVariablesAsync(resolvedType?)
await figma.variables.getVariableCollectionByIdAsync(id)
await figma.variables.getLocalVariableCollectionsAsync()

// Binding variables to paints (COLOR variables)
const newPaint = figma.variables.setBoundVariableForPaint(paintCopy, "color", variable)
// ⚠️ Returns a NEW paint — must capture return value!
node.fills = [newPaint]

// Binding variables to effects (COLOR/FLOAT variables)
const newEffect = figma.variables.setBoundVariableForEffect(effectCopy, field, variable)
// field for shadows: "color" (COLOR), "radius" | "spread" | "offsetX" | "offsetY" (FLOAT)
// field for blurs: "radius" (FLOAT)
// ⚠️ Returns a NEW effect — must capture return value!
node.effects = [newEffect]

// Binding variables to layout grids (FLOAT variables)
const newGrid = figma.variables.setBoundVariableForLayoutGrid(gridCopy, field, variable)
// field: "sectionSize" | "offset" | "count" | "gutterSize"
// ⚠️ Returns a NEW layout grid — must capture return value!
node.layoutGrids = [newGrid]

// Binding variables to node properties (FLOAT/STRING/BOOLEAN)
// Layout & sizing (FLOAT):
node.setBoundVariable("width", variable)
node.setBoundVariable("height", variable)
node.setBoundVariable("minWidth", variable)
node.setBoundVariable("maxWidth", variable)
node.setBoundVariable("minHeight", variable)
node.setBoundVariable("maxHeight", variable)
node.setBoundVariable("paddingLeft", variable)
node.setBoundVariable("paddingRight", variable)
node.setBoundVariable("paddingTop", variable)
node.setBoundVariable("paddingBottom", variable)
node.setBoundVariable("itemSpacing", variable)
node.setBoundVariable("counterAxisSpacing", variable)
// Corner radii (FLOAT) — use individual corners, NOT cornerRadius:
node.setBoundVariable("topLeftRadius", variable)
node.setBoundVariable("topRightRadius", variable)
node.setBoundVariable("bottomLeftRadius", variable)
node.setBoundVariable("bottomRightRadius", variable)
// Other (FLOAT):
node.setBoundVariable("opacity", variable)
node.setBoundVariable("strokeWeight", variable)
// ⚠️ fontSize, fontWeight, lineHeight are NOT bindable via setBoundVariable
// — set these directly as values on text nodes

// Aliases
figma.variables.createVariableAlias(variable)

// Explicit modes — CRITICAL for variant components
node.setExplicitVariableModeForCollection(collection, modeId)  // pass collection object, NOT an ID string
// Without this, all nodes use the default (first) mode of the collection
```

## Core Properties

```js
figma.root                      // DocumentNode
figma.currentPage               // Current page (read-only in use_figma; sync setter throws)
figma.setCurrentPageAsync(page) // Switch page and load its content (MUST await)
figma.fileKey                   // File key string
figma.mixed                     // Mixed sentinel value
```

## Node Manipulation

```js
// Fills & Strokes (read-only arrays — must clone)
node.fills = [{ type: 'SOLID', color: { r: 1, g: 0, b: 0 } }]
node.strokes = [{ type: 'SOLID', color: { r: 0, g: 0, b: 0 } }]
node.strokeWeight = 1
node.strokeAlign = 'INSIDE'             // 'INSIDE' | 'CENTER' | 'OUTSIDE'

// Effects
node.effects = [{ type: 'DROP_SHADOW', color: {r:0,g:0,b:0,a:0.25}, offset:{x:0,y:4}, radius:4, visible:true }]

// Layout
node.layoutMode = 'HORIZONTAL'          // 'NONE' | 'HORIZONTAL' | 'VERTICAL'
node.primaryAxisAlignItems = 'CENTER'    // 'MIN' | 'CENTER' | 'MAX' | 'SPACE_BETWEEN'
node.counterAxisAlignItems = 'CENTER'    // 'MIN' | 'CENTER' | 'MAX' | 'BASELINE'
node.paddingLeft = 8
node.paddingRight = 8
node.paddingTop = 4
node.paddingBottom = 4
node.itemSpacing = 4
node.layoutSizingHorizontal = 'HUG'     // 'FIXED' | 'HUG' | 'FILL'
node.layoutSizingVertical = 'HUG'       // 'FIXED' | 'HUG' | 'FILL'

// Sizing
node.resize(width, height)                     // ⚠️ Resets sizing modes to FIXED
node.resizeWithoutConstraints(width, height)   // Doesn't affect constraints

// Corner radius
node.cornerRadius = 8

// Visibility & Opacity
node.visible = true
node.opacity = 0.5

// Naming & Hierarchy
node.name = "My Node"
parent.appendChild(child)
parent.insertChild(index, child)
node.remove()
```

## Descriptions & Documentation Links

```js
// Description — plain text, shown in Figma's component panel
node.description = "A short summary of this component's purpose and usage."

// Documentation links — array of {uri, label} shown as clickable links
componentSet.documentationLinks = [
  { uri: "https://example.com/docs", label: "Component Docs" }
]
// ⚠️ uri MUST be a valid URL (https://...) — relative paths will throw
```

## SVG Import

```js
const svgNode = figma.createNodeFromSvg('<svg>...</svg>')
```

## Images

```js
const image = figma.createImage(uint8Array)
node.fills = [{ type: 'IMAGE', scaleMode: 'FILL', imageHash: image.hash }]
```

## Utilities

```js
figma.base64Encode(uint8Array)     // Uint8Array → base64 string
figma.base64Decode(base64String)   // base64 string → Uint8Array
figma.createComponentFromNode(node) // Convert existing node to component (Design/Sites only)
```

## Plugin Lifecycle

Scripts are automatically wrapped in an async IIFE with error handling. Use `return` to send data back:

```js
return { nodeId: frame.id }     // Return object — auto-serialized to JSON
return "success message"        // Return string
// Errors are auto-captured — no try/catch or closePlugin needed
```

## Node Traversal

```js
node.findAll(pred?)            // Find all descendants matching predicate
node.findOne(pred?)            // Find first descendant matching predicate
node.findChildren(pred?)       // Find direct children matching predicate
node.findChild(pred?)          // Find first direct child matching predicate
node.children                  // Direct children array
node.parent                    // Parent node
```

---

## What Does NOT Work

| API | Status |
|-----|--------|
| `figma.notify()` | **Throws "not implemented"** — most common mistake |
| `figma.showUI()` | No-op (silently ignored) |
| `figma.openExternal()` | No-op (silently ignored) |
| `figma.listAvailableFontsAsync()` | Not implemented |
| `figma.loadAllPagesAsync()` | Not implemented |
| `figma.variables.extendLibraryCollectionByKeyAsync()` | Not implemented |
| `figma.teamLibrary.*` | Not implemented (requires LiveGraph) |
