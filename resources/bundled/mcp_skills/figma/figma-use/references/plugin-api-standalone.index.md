# Plugin API Index

> Full typings: `plugin-api-standalone.d.ts` (11,292 lines)  
> Grep by symbol name to jump to definition. All `L#` line numbers refer to that file.

---

## figma.\* — PluginAPI (L24)

### Identity & State

| Member                          | Type                                                                             |
| ------------------------------- | -------------------------------------------------------------------------------- |
| `apiVersion`                    | `'1.0.0'`                                                                        |
| `editorType`                    | `'figma' \| 'figjam' \| 'dev' \| 'slides' \| 'buzz'`                             |
| `mode`                          | `'default' \| 'textreview' \| 'inspect' \| 'codegen' \| 'linkpreview' \| 'auth'` |
| `fileKey`                       | `string \| undefined`                                                            |
| `root`                          | `DocumentNode`                                                                   |
| `currentPage`                   | `PageNode` — assign via `setCurrentPageAsync`                                    |
| `currentUser`                   | `User \| null`                                                                   |
| `mixed`                         | `unique symbol` — sentinel for mixed values in selection                         |
| `skipInvisibleInstanceChildren` | `boolean`                                                                        |

### Navigation & Lookup

| Method                      | Returns                                                 |
| --------------------------- | ------------------------------------------------------- |
| `setCurrentPageAsync(page)` | `Promise<void>` — **MUST use this**; sync setter throws |
| `getNodeByIdAsync(id)`      | `Promise<BaseNode \| null>`                             |
| `getNodeById(id)`           | `BaseNode \| null`                                      |
| `getStyleByIdAsync(id)`     | `Promise<BaseStyle \| null>`                            |
| `getStyleById(id)`          | `BaseStyle \| null`                                     |

### Create Nodes

| Method                              | Returns                     |
| ----------------------------------- | --------------------------- |
| `createFrame()`                     | `FrameNode`                 |
| `createComponent()`                 | `ComponentNode`             |
| `createComponentFromNode(node)`     | `ComponentNode`             |
| `createRectangle()`                 | `RectangleNode`             |
| `createEllipse()`                   | `EllipseNode`               |
| `createLine()`                      | `LineNode`                  |
| `createPolygon()`                   | `PolygonNode`               |
| `createStar()`                      | `StarNode`                  |
| `createVector()`                    | `VectorNode`                |
| `createText()`                      | `TextNode`                  |
| `createSection()`                   | `SectionNode`               |
| `createPage()`                      | `PageNode`                  |
| `createSlice()`                     | `SliceNode`                 |
| `createBooleanOperation()`          | `BooleanOperationNode`      |
| `createTable(rows?, cols?)`         | `TableNode`                 |
| `createImage(data: Uint8Array)`     | `Image`                     |
| `createNodeFromSvg(svg)`            | `FrameNode`                 |
| `createNodeFromJSXAsync(jsx)`       | `Promise<SceneNode>`        |
| `importComponentByKeyAsync(key)`    | `Promise<ComponentNode>`    |
| `importComponentSetByKeyAsync(key)` | `Promise<ComponentSetNode>` |
| `importStyleByKeyAsync(key)`        | `Promise<BaseStyle>`        |

### Styles (Local)

| Method                             | Returns         |
| ---------------------------------- | --------------- |
| `createPaintStyle()`               | `PaintStyle`    |
| `createTextStyle()`                | `TextStyle`     |
| `createEffectStyle()`              | `EffectStyle`   |
| `createGridStyle()`                | `GridStyle`     |
| `getLocalPaintStyles()` / `Async`  | `PaintStyle[]`  |
| `getLocalTextStyles()` / `Async`   | `TextStyle[]`   |
| `getLocalEffectStyles()` / `Async` | `EffectStyle[]` |
| `getLocalGridStyles()` / `Async`   | `GridStyle[]`   |

### Fonts

| Method                      | Notes                              |
| --------------------------- | ---------------------------------- |
| `loadFontAsync(fontName)`   | **MUST call before any text edit** |
| `listAvailableFontsAsync()` | `Promise<Font[]>`                  |
| `hasMissingFont`            | `boolean`                          |

### Plugin Lifecycle

| Method                                  | Notes                                                        |
| --------------------------------------- | ------------------------------------------------------------ |
| `closePlugin(message?)`                 | Auto-called; use `return` instead to pass results back       |
| `closePluginWithFailure(message?)`      | Auto-called on errors; do not call manually                  |
| `commitUndo()`                          | Snapshot to undo history                                     |
| `triggerUndo()`                         | Revert to last snapshot                                      |
| `saveVersionHistoryAsync(title, desc?)` | `Promise<VersionHistoryResult>`                              |
| `notify(message, options?)`             | **throws "not implemented" in use_figma — do not use** |
| `openExternal(url)`                     | Opens URL in browser                                         |

### Sub-APIs (properties on figma)

| Property              | Interface                | L#    |
| --------------------- | ------------------------ | ----- |
| `figma.variables`     | `VariablesAPI`           | L2016 |
| `figma.ui`            | `UIAPI`                  | L2604 |
| `figma.util`          | `UtilAPI`                | L2691 |
| `figma.constants`     | `ConstantsAPI`           | L2809 |
| `figma.clientStorage` | `ClientStorageAPI`       | L2531 |
| `figma.viewport`      | `ViewportAPI`            | L3086 |
| `figma.parameters`    | `ParametersAPI`          | L3292 |
| `figma.teamLibrary`   | `TeamLibraryAPI`         | L2372 |
| `figma.annotations`   | `AnnotationsAPI`         | L2187 |
| `figma.codegen`       | `CodegenAPI`             | L2871 |
| `figma.textreview?`   | `TextReviewAPI`          | L3166 |
| `figma.payments?`     | `PaymentsAPI`            | L2420 |
| `figma.buzz`          | `BuzzAPI`                | L2211 |
| `figma.timer?`        | `TimerAPI` (FigJam only) | L3053 |

---

## VariablesAPI — figma.variables (L2016)

```
getVariableByIdAsync(id)                 Promise<Variable | null>    ← preferred; sync deprecated
getVariableCollectionByIdAsync(id)       Promise<VariableCollection | null>    ← preferred; sync deprecated
getLocalVariablesAsync(type?)            Promise<Variable[]>         ← preferred; filter by VariableResolvedDataType; sync deprecated
getLocalVariableCollectionsAsync()       Promise<VariableCollection[]>    ← preferred; sync deprecated
createVariable(name, collection, type)   Variable
createVariableCollection(name)           VariableCollection
createVariableAlias(variable)            VariableAlias
importVariableByKeyAsync(key)            Promise<Variable>
setBoundVariableForPaint(paint, field, variable)    → returns NEW paint — reassign
setBoundVariableForEffect(effect, field, variable)  → returns NEW effect — reassign
setBoundVariableForLayoutGrid(grid, field, variable)
```

**Variable (L10204):** `name`, `resolvedType`, `codeSyntax`, `scopes`, `hiddenFromPublishing`, `valuesByMode`, `variableCollectionId`

- `setVariableCodeSyntax(platform, value)` — platform: `'WEB' | 'ANDROID' | 'iOS'`
- `setValueForMode(collectionId, modeId, value)`
- `remove()`

**VariableCollection (L10418):** `name`, `modes`, `variableIds`, `defaultModeId`, `hiddenFromPublishing`

- `addMode(name)` → `modeId`; `removeMode(modeId)`; `renameMode(modeId, name)`

---

## Node Types

### Concrete Scene Nodes

| Node                   | L#     | Key characteristics                                |
| ---------------------- | ------ | -------------------------------------------------- |
| `DocumentNode`         | L8960  | Root; `children: PageNode[]`                       |
| `PageNode`             | L9119  | `children`, local styles, `backgrounds`            |
| `FrameNode`            | L9311  | `DefaultFrameMixin` — auto-layout, clips, children |
| `GroupNode`            | L9321  | Children only, no auto-layout                      |
| `ComponentNode`        | L9678  | Like Frame + publishable                           |
| `ComponentSetNode`     | L9653  | Variant set container                              |
| `InstanceNode`         | L9719  | Like Frame; `mainComponent`, `detach()`            |
| `RectangleNode`        | L9378  | `DefaultShapeMixin` + corners                      |
| `EllipseNode`          | L9410  | + `arcData`                                        |
| `LineNode`             | L9396  |                                                    |
| `PolygonNode`          | L9430  |                                                    |
| `StarNode`             | L9450  |                                                    |
| `VectorNode`           | L9476  | Vector paths                                       |
| `TextNode`             | L9493  | Rich text, fonts, segments                         |
| `TextPathNode`         | L9564  | Text along path                                    |
| `BooleanOperationNode` | L9792  | `booleanOperation` property                        |
| `SliceNode`            | L9368  | Export only                                        |
| `SectionNode`          | L10754 | Grouping + fills                                   |
| `TableNode`            | L9862  | `TableCellNode` children                           |

**FigJam only:** `StickyNode` L9812, `ConnectorNode` L10121, `ShapeWithTextNode` L9999, `StampNode` L9838, `CodeBlockNode` L10080, `EmbedNode` L10661, `LinkUnfurlNode` L10701, `MediaNode` L10721

**Slides only:** `SlideNode` L10784, `SlideRowNode` L10809, `SlideGridNode` L10822

**Union types:**

```
type SceneNode  (L10917) = FrameNode | GroupNode | SliceNode | RectangleNode | LineNode
  | EllipseNode | PolygonNode | StarNode | VectorNode | TextNode | ComponentSetNode
  | ComponentNode | InstanceNode | BooleanOperationNode | SectionNode | ...
type BaseNode   (L10913) = DocumentNode | PageNode | SceneNode
```

---

## Mixin Interfaces

| Mixin                        | L#    | Provides                                                                                        |
| ---------------------------- | ----- | ----------------------------------------------------------------------------------------------- |
| `BaseNodeMixin`              | L5284 | `id`, `name`, `type`, `parent`, `remove()`, plugin data                                         |
| `SceneNodeMixin`             | L5561 | `visible`, `locked`, `opacity`, variable bindings                                               |
| `ChildrenMixin`              | L5773 | `children`, `appendChild()`, `insertChild()`, `findAll()`, `findOne()`, `findAllWithCriteria()` |
| `LayoutMixin`                | L6135 | `x`, `y`, `width`, `height`, `rotation`, `resize()`, `rescale()`                                |
| `AutoLayoutMixin`            | L6436 | `layoutMode`, axis alignment, padding, `itemSpacing`, `layoutSizingHorizontal/Vertical`         |
| `AutoLayoutChildrenMixin`    | L7064 | `layoutAlign`, `layoutGrow`, sizing — **set AFTER `appendChild()`**                             |
| `GridLayoutMixin`            | L6939 | CSS Grid tracks, gap, template                                                                  |
| `GridChildrenMixin`          | L7127 | grid child positioning                                                                          |
| `GeometryMixin`              | L7485 | `fills`, `strokes`, `strokeWeight`, `strokeAlign`                                               |
| `MinimalFillsMixin`          | L7328 | `fills` only                                                                                    |
| `MinimalStrokesMixin`        | L7246 | `strokes`, `strokeWeight`                                                                       |
| `BlendMixin`                 | L6339 | `opacity`, `blendMode`, `isMask`, `effects`                                                     |
| `CornerMixin`                | L7537 | `cornerRadius`, `cornerSmoothing`                                                               |
| `RectangleCornerMixin`       | L7560 | Per-corner radii                                                                                |
| `ExportMixin`                | L7577 | `exportSettings`, `exportAsync()`                                                               |
| `ReactionMixin`              | L7704 | `reactions` (prototyping)                                                                       |
| `PublishableMixin`           | L7875 | `description`, `key`, `getPublishStatusAsync()`                                                 |
| `VariantMixin`               | L8182 | `variantProperties`                                                                             |
| `ComponentPropertiesMixin`   | L8229 | `componentProperties`, `addComponentProperty()`                                                 |
| `PluginDataMixin`            | L5443 | `getSharedPluginData()`, `setSharedPluginData()` supported; `getPluginData()`, `setPluginData()` **NOT supported** |
| `FramePrototypingMixin`      | L7651 | `overflowDirection`, `numberOfFixedChildren`                                                    |
| `BaseFrameMixin`             | L7939 | ChildrenMixin + LayoutMixin + AutoLayoutMixin + GeometryMixin + …                               |
| `DefaultFrameMixin`          | L7997 | BaseFrameMixin + FramePrototypingMixin + ReactionMixin                                          |
| `DefaultShapeMixin`          | L7928 | BlendMixin + GeometryMixin + LayoutMixin + ExportMixin + ReactionMixin                          |
| `ExplicitVariableModesMixin` | L9084 | `setExplicitVariableModeForCollection()`                                                        |

---

## Paint & Fill (L4302)

| Type            | L#    | Notes                                                                             |
| --------------- | ----- | --------------------------------------------------------------------------------- |
| `SolidPaint`    | L4302 | `type:'SOLID'`, `color: RGB`, `opacity`, `visible`, `blendMode`                   |
| `GradientPaint` | L4357 | `type: 'GRADIENT_LINEAR\|RADIAL\|ANGULAR\|DIAMOND'`, `gradientStops: ColorStop[]` |
| `ImagePaint`    | L4377 | `type:'IMAGE'`, `imageHash`, `scaleMode`                                          |
| `VideoPaint`    | L4413 | `type:'VIDEO'`                                                                    |
| `PatternPaint`  | L4449 | `type:'PATTERN'`                                                                  |
| `type Paint`    | L4481 | Union of all five                                                                 |
| `ColorStop`     | L4271 | `{ position: number, color: RGBA }`                                               |
| `ImageFilters`  | L4290 | exposure, contrast, saturation, etc.                                              |

> **CRITICAL**: Fills/strokes are **read-only arrays** — clone, modify, reassign.

---

## Effects (L3966)

| Type                               | L#    |
| ---------------------------------- | ----- |
| `DropShadowEffect`                 | L3966 |
| `InnerShadowEffect`                | L4009 |
| `BlurEffect` (Normal/Progressive)  | L4048 |
| `NoiseEffect` (Mono/Duo/Multitone) | L4105 |
| `TextureEffect`                    | L4180 |
| `GlassEffect`                      | L4209 |
| `type Effect`                      | L4250 |

---

## Typography

| Type                | L#    | Notes                                                                                  |
| ------------------- | ----- | -------------------------------------------------------------------------------------- |
| `FontName`          | L3697 | `{ family: string, style: string }`                                                    |
| `TextNode`          | L9493 | `characters`, `textAlignHorizontal`, `fontSize`, `fontName`, `getStyledTextSegments()` |
| `StyledTextSegment` | L4882 | Per-range text properties                                                              |
| `LetterSpacing`     | L4826 | `{ value, unit: 'PIXELS'\|'PERCENT' }`                                                 |
| `LineHeight`        | L4830 | `{ value, unit } \| { unit: 'AUTO' }`                                                  |
| `TextCase`          | L3701 | `'ORIGINAL'\|'UPPER'\|'LOWER'\|'TITLE'\|'SMALL_CAPS'`                                  |
| `TextDecoration`    | L3702 | `'NONE'\|'UNDERLINE'\|'STRIKETHROUGH'`                                                 |
| `OpenTypeFeature`   | L3728 | Ligatures, numerals, etc.                                                              |

---

## Variables & Bindings

| Type                          | L#     | Notes                                                         |
| ----------------------------- | ------ | ------------------------------------------------------------- |
| `Variable`                    | L10204 | Core variable object                                          |
| `VariableCollection`          | L10418 | Collection of variables + modes                               |
| `VariableAlias`               | L10172 | Reference to another variable                                 |
| `VariableValue`               | L10176 | `boolean \| string \| number \| RGB \| RGBA \| VariableAlias` |
| `VariableResolvedDataType`    | L10171 | `'BOOLEAN' \| 'COLOR' \| 'FLOAT' \| 'STRING'`                 |
| `VariableDataType`            | L5023  | Includes `'VARIABLE_ALIAS' \| 'EXPRESSION'`                   |
| `VariableScope`               | L10177 | Where variable can be applied                                 |
| `CodeSyntaxPlatform`          | L10203 | `'WEB' \| 'ANDROID' \| 'iOS'`                                 |
| `VariableBindableNodeField`   | L5712  | Node fields that accept variable binding                      |
| `VariableBindableTextField`   | L5739  | Text-specific bindable fields                                 |
| `VariableBindablePaintField`  | L5748  | `'color'`                                                     |
| `VariableBindableEffectField` | L5751  | `'color'\|'radius'\|'spread'\|'offsetX'\|'offsetY'`           |

---

## Styles

| Interface        | L#     | Notes                                                  |
| ---------------- | ------ | ------------------------------------------------------ |
| `BaseStyleMixin` | L10977 | `name`, `id`, `key`, `type`, `description`, `remove()` |
| `PaintStyle`     | L11002 | `type:'PAINT'`, `paints: Paint[]`                      |
| `TextStyle`      | L11018 | `type:'TEXT'`, font properties                         |
| `EffectStyle`    | L11087 | `type:'EFFECT'`, `effects: Effect[]`                   |
| `GridStyle`      | L11103 | `type:'GRID'`, `layoutGrids`                           |
| `type BaseStyle` | L11119 | Union of all four                                      |
| `type StyleType` | L10955 | `'PAINT' \| 'TEXT' \| 'EFFECT' \| 'GRID'`              |

---

## Primitives & Geometry

| Type             | L#    | Shape                                         |
| ---------------- | ----- | --------------------------------------------- |
| `Vector`         | L3667 | `{ x: number, y: number }`                    |
| `Rect`           | L3671 | `{ x, y, width, height }`                     |
| `RGB`            | L3680 | `{ r, g, b }` — **0–1 range, not 0–255**      |
| `RGBA`           | L3688 | `{ r, g, b, a }` — **0–1 range**              |
| `Transform`      | L3666 | `[[a,b,tx],[c,d,ty]]` 2×3 affine matrix       |
| `ArcData`        | L3958 | `{ startingAngle, endingAngle, innerRadius }` |
| `Constraints`    | L4264 | `{ horizontal, vertical }: ConstraintType`    |
| `ConstraintType` | L4260 | `'MIN'\|'CENTER'\|'MAX'\|'STRETCH'\|'SCALE'`  |
| `VectorPath`     | L4792 | `{ windingRule, data: string }`               |
| `VectorNetwork`  | L4775 | vertices + segments + regions                 |
| `Guide`          | L4482 | `{ axis, offset }`                            |

---

## Prototyping

| Type                  | L#    | Notes                                                     |
| --------------------- | ----- | --------------------------------------------------------- |
| `Reaction`            | L5015 | trigger + action pair                                     |
| `Trigger`             | L5146 | what initiates the reaction                               |
| `Action`              | L5064 | what happens                                              |
| `Transition`          | L5145 | `SimpleTransition \| DirectionalTransition`               |
| `Easing`              | L5182 | easing curve definition                                   |
| `Navigation`          | L5178 | `'NAVIGATE'\|'SWAP'\|'OVERLAY'\|'SCROLL_TO'\|'CHANGE_TO'` |
| `OverflowDirection`   | L5215 | `'NONE'\|'HORIZONTAL'\|'VERTICAL'\|'BOTH'`                |
| `OverlayPositionType` | L5219 | overlay placement                                         |

---

## Events & Changes

| Type                  | L#    | Notes                                                           |
| --------------------- | ----- | --------------------------------------------------------------- |
| `ArgFreeEventType`    | L11   | `'selectionchange'\|'currentpagechange'\|'close'\|timer events` |
| `RunEvent`            | L3321 | plugin run with parameters                                      |
| `DropEvent`           | L3339 | drag-and-drop                                                   |
| `DocumentChangeEvent` | L3359 | any document change                                             |
| `NodeChangeEvent`     | L3626 | node property changes                                           |
| `NodeChangeProperty`  | L3499 | all watchable property names                                    |
| `StyleChangeEvent`    | L3365 | style create/delete/update                                      |
| `DocumentChange`      | L3489 | `CreateChange \| DeleteChange \| PropertyChange`                |
| `TextReviewEvent`     | L3657 | text review mode                                                |

---

## Export

| Type                        | L#    | Notes                                         |
| --------------------------- | ----- | --------------------------------------------- |
| `ExportSettingsImage`       | L4561 | PNG/JPG/WEBP/BMP                              |
| `ExportSettingsSVG`         | L4634 |                                               |
| `ExportSettingsPDF`         | L4653 |                                               |
| `ExportSettingsREST`        | L4667 |                                               |
| `ExportSettingsConstraints` | L4554 | `{ type: 'SCALE'\|'WIDTH'\|'HEIGHT', value }` |

---

## Key Sub-API Surfaces

**ClientStorageAPI (L2531):** `getAsync(key)`, `setAsync(key, value)`, `keysAsync()`, `deleteAsync(key)`

**ViewportAPI (L3086):** `center: Vector`, `zoom: number`, `scrollAndZoomIntoView(nodes)`, `bounds: Rect`

**UtilAPI (L2691):** `solidPaint(hex, opacity?)`, `rgba(r,g,b,a?)`, `rgb(r,g,b)`, `colorToHex(color)`, `loadImageAsync(url)`, `clone(val)`

**TeamLibraryAPI (L2372):** `getAvailableLibraryVariableCollectionsAsync()`, `importVariableByKeyAsync(key)`

**Image (L11120):** `hash`, `getBytesAsync()`, `getSizeAsync()`

---

## All Symbols (flat — grep these against the .d.ts file)

To find any symbol: `grep -n "^interface Foo\|^type Foo\|^declare type Foo" plugin-api-standalone.d.ts`

```
PluginAPI               VariablesAPI            AnnotationsAPI          TeamLibraryAPI
UIAPI                   UtilAPI                 ViewportAPI             ClientStorageAPI
ConstantsAPI            CodegenAPI              PaymentsAPI             TextReviewAPI
ParametersAPI           TimerAPI                BuzzAPI                 DevResourcesAPI

DocumentNode            PageNode                FrameNode               GroupNode
ComponentNode           ComponentSetNode        InstanceNode            RectangleNode
EllipseNode             LineNode                PolygonNode             StarNode
VectorNode              TextNode                TextPathNode            BooleanOperationNode
SliceNode               SectionNode             TableNode               TableCellNode
StickyNode              ConnectorNode           ShapeWithTextNode       StampNode
CodeBlockNode           EmbedNode               LinkUnfurlNode          MediaNode
WidgetNode              SlideNode               SlideRowNode            SlideGridNode
TransformGroupNode      HighlightNode           WashiTapeNode

BaseNodeMixin           SceneNodeMixin          ChildrenMixin           LayoutMixin
AutoLayoutMixin         AutoLayoutChildrenMixin GridLayoutMixin         GridChildrenMixin
GeometryMixin           MinimalFillsMixin       MinimalStrokesMixin     BlendMixin
MinimalBlendMixin       CornerMixin             RectangleCornerMixin    ExportMixin
ReactionMixin           PublishableMixin        VariantMixin            ComponentPropertiesMixin
PluginDataMixin         DevResourcesMixin       DevStatusMixin          StickableMixin
ConstraintMixin         DimensionAndPositionMixin AspectRatioLockMixin  FramePrototypingMixin
BaseFrameMixin          DefaultFrameMixin       DefaultShapeMixin       OpaqueNodeMixin
VectorLikeMixin         ComplexStrokesMixin     IndividualStrokesMixin  ContainerMixin
AnnotationsMixin        MeasurementsMixin       ExplicitVariableModesMixin

Variable                VariableCollection      VariableAlias           ExtendedVariableCollection
LibraryVariableCollection LibraryVariable
VariableValue           VariableResolvedDataType VariableDataType       VariableScope
CodeSyntaxPlatform      VariableBindableNodeField VariableBindableTextField
VariableBindablePaintField VariableBindableEffectField VariableBindableLayoutGridField

SolidPaint              GradientPaint           ImagePaint              VideoPaint
PatternPaint            Paint                   ColorStop               ImageFilters
DropShadowEffect        InnerShadowEffect       BlurEffect              NoiseEffect
TextureEffect           GlassEffect             Effect
LayoutGrid              RowsColsLayoutGrid      GridLayoutGrid

PaintStyle              TextStyle               EffectStyle             GridStyle
BaseStyle               BaseStyleMixin          StyleType

FontName                Font                    LetterSpacing           LineHeight
TextCase                TextDecoration          TextDecorationStyle     FontStyle
OpenTypeFeature         StyledTextSegment       LeadingTrim

Vector                  Rect                    RGB                     RGBA
Transform               ArcData                 Constraints             ConstraintType
VectorPath              VectorNetwork           VectorVertex            VectorSegment
VectorRegion            Guide                   BlendMode               MaskType

Reaction                Trigger                 Action                  Transition
Easing                  Navigation              OverflowDirection       OverlayPositionType
OverlayBackground       PublishStatus

ArgFreeEventType        RunEvent                DropEvent               DocumentChangeEvent
NodeChangeEvent         NodeChangeProperty      StyleChangeEvent        DocumentChange
TextReviewEvent         SlidesViewChangeEvent   CanvasViewChangeEvent

ExportSettingsImage     ExportSettingsSVG       ExportSettingsPDF       ExportSettingsREST
ExportSettingsConstraints

User                    ActiveUser              BaseUser                Image
Video                   VersionHistoryResult    FindAllCriteria
```
