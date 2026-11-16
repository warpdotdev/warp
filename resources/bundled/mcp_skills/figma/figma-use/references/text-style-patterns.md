# Text Style API Patterns

> Part of the [use_figma skill](../SKILL.md). How to create, apply, and inspect text styles using the Plugin API.
>
> For design system context (when to create text styles, how they relate to tokens, headless limitations), see [wwds-text-styles](working-with-design-systems/wwds-text-styles.md).

## Contents

- Listing Text Styles
- Creating a Text Style
- Probing Font Styles
- Creating a Type Ramp (Multi-Step)
- Importing Library Text Styles
- Applying Text Styles to Nodes

## Listing Text Styles

```javascript
/**
 * Lists all local text styles with their key properties.
 *
 * @returns {Promise<Array<{id: string, name: string, key: string, fontSize: number, fontName: FontName, lineHeight: LineHeight, letterSpacing: LetterSpacing}>>}
 */
async function listTextStyles() {
  const styles = await figma.getLocalTextStylesAsync();
  return styles.map(s => ({
    id: s.id,
    name: s.name,
    key: s.key,
    fontSize: s.fontSize,
    fontName: s.fontName,
    lineHeight: s.lineHeight,
    letterSpacing: s.letterSpacing
  }));
}
```

Full runnable script:

```javascript
const results = await listTextStyles();
return results;
```

## Creating a Text Style

Font **MUST** be loaded before setting `fontName`. `lineHeight` and `letterSpacing` must be `{value, unit}` objects — bare numbers throw.

```javascript
/**
 * Creates a text style with all typographic properties set.
 * Font MUST be loaded before calling.
 *
 * @param {string} name - Slash-delimited name, e.g. "body/base"
 * @param {{ family: string, style: string }} fontName
 * @param {number} fontSize - In pixels
 * @param {{ value: number, unit: 'PIXELS' | 'PERCENT' } | { unit: 'AUTO' }} lineHeight
 * @param {{ value: number, unit: 'PIXELS' | 'PERCENT' }} [letterSpacing]
 * @param {string} [description] - e.g. the CSS variable name "CSS: var(--font-body-base)"
 * @returns {TextStyle}
 */
function createTextStyleFull(name, fontName, fontSize, lineHeight, letterSpacing, description) {
  const style = figma.createTextStyle();
  style.name = name;
  style.fontName = fontName;
  style.fontSize = fontSize;
  style.lineHeight = lineHeight; // { unit: 'AUTO' } | { value, unit: 'PIXELS'|'PERCENT' }
  if (letterSpacing) style.letterSpacing = letterSpacing;
  if (description) style.description = description;
  return style;
}
```

## Probing Font Styles

Font style names vary per provider and per file (`"SemiBold"` vs `"Semi Bold"`). Always probe before hardcoding:

```javascript
/**
 * Probes available font styles for a given family.
 * Useful when font style names are unknown (e.g. "SemiBold" vs "Semi Bold").
 *
 * @param {string} family - Font family name, e.g. "Inter"
 * @param {string[]} stylesToTest - Candidate style names to probe
 * @returns {Promise<string[]>} - Style names that loaded successfully
 */
async function probeAvailableFontStyles(family, stylesToTest) {
  const available = [];
  for (const style of stylesToTest) {
    try {
      await figma.loadFontAsync({ family, style });
      available.push(style);
    } catch (_) {}
  }
  return available;
}
```

## Creating a Type Ramp (Multi-Step)

Handles font loading, deduplication, and idempotency. Each entry: `[name, fontFamily, fontStyle, fontSize_px, lineHeight, cssVar]`.

**HEADLESS NOTE:** `setBoundVariable` on `TextStyle` is not supported in `use_figma`. This function sets raw values. To bind variables, do it interactively in Figma after creation.

```javascript
/**
 * Creates a full type ramp from a token definition array.
 * Handles font loading, deduplication, and idempotency.
 *
 * Each entry: [name, fontFamily, fontStyle, fontSize_px, lineHeight, cssVar]
 *   - lineHeight: { unit: 'AUTO' } or { value: number, unit: 'PIXELS' | 'PERCENT' }
 *
 * @param {Array} defs - Array of [name, fontFamily, fontStyle, fontSize, lineHeight, cssVar] tuples
 * @returns {Promise<{ created: string[], skipped: string[] }>}
 */
async function createTypeRamp(defs) {
  const uniqueFonts = new Set();
  for (const [, family, style] of defs) {
    uniqueFonts.add(JSON.stringify({ family, style }));
  }
  await Promise.all(
    [...uniqueFonts].map(f => figma.loadFontAsync(JSON.parse(f)))
  );

  const existing = new Set(
    (await figma.getLocalTextStylesAsync()).map(s => s.name)
  );

  const created = [];
  const skipped = [];

  for (const [name, family, style, fontSize, lineHeight, cssVar] of defs) {
    if (existing.has(name)) {
      skipped.push(name);
      continue;
    }
    const ts = figma.createTextStyle();
    ts.name = name;
    ts.fontName = { family, style };
    ts.fontSize = fontSize;
    ts.lineHeight = lineHeight ?? { unit: 'AUTO' };
    if (cssVar) ts.description = `CSS: var(${cssVar})`;
    created.push(name);
  }

  return { created, skipped };
}
```

Full runnable script:

```javascript
const defs = [
  ['heading/xl', 'Inter', 'Bold',      48, { unit: 'PIXELS', value: 56 }, '--font-heading-xl'],
  ['heading/lg', 'Inter', 'Bold',      36, { unit: 'PIXELS', value: 44 }, '--font-heading-lg'],
  ['body/base',  'Inter', 'Regular',   16, { unit: 'AUTO' },              '--font-body-base'],
  ['body/sm',    'Inter', 'Regular',   14, { unit: 'AUTO' },              '--font-body-sm'],
  ['code/base',  'Roboto Mono', 'Regular', 14, { unit: 'AUTO' },          '--font-code-base'],
];
const result = await createTypeRamp(defs);
return result;
```

## Importing Library Text Styles

For text styles from **team libraries**, use `importStyleByKeyAsync`:

```javascript
// Import a library text style by key
const headingStyle = await figma.importStyleByKeyAsync("TEXT_STYLE_KEY");
// Apply to a text node
await textNode.setTextStyleIdAsync(headingStyle.id);
```

`search_design_system` with `includeStyles: true` returns style keys you can import this way. Prefer importing library styles over creating new ones.

## Applying Text Styles to Nodes

```javascript
/**
 * Applies a text style to all TEXT nodes on the current page that match a given name pattern.
 *
 * @param {string} styleId - The ID of a TextStyle.
 * @param {string} nodeNamePattern - Substring match against node names.
 * @returns {Promise<number>} - Number of nodes the style was applied to.
 */
async function applyTextStyleToMatchingNodes(styleId, nodeNamePattern) {
  const textNodes = figma.currentPage.findAllWithCriteria({ types: ['TEXT'] });
  let applied = 0;
  for (const node of textNodes) {
    if (node.name.includes(nodeNamePattern)) {
      await node.setTextStyleIdAsync(styleId);
      applied++;
    }
  }
  return applied;
}
```

Full runnable script:

```javascript
const applied = await applyTextStyleToMatchingNodes('STYLE_ID', 'Heading');
return { applied };
```
