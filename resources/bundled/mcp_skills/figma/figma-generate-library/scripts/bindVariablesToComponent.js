/**
 * bindVariablesToComponent
 *
 * Binds design token variables to the visual properties of a component node.
 * Supports fills, strokes, all padding directions, item spacing, and corner radius.
 * Only binds properties for which a variable ID is provided in `bindings`.
 *
 * This function should be called on each variant individually within a component
 * set, OR on the component set itself for properties shared by all variants.
 *
 * @param {ComponentNode | FrameNode | RectangleNode} component
 *   The Figma node to mutate. Usually a ComponentNode or one of its children.
 * @param {{
 *   fills?: string,
 *   strokes?: string,
 *   paddingTop?: string,
 *   paddingBottom?: string,
 *   paddingLeft?: string,
 *   paddingRight?: string,
 *   itemSpacing?: string,
 *   cornerRadius?: string
 * }} bindings
 *   Each key is a visual property name; each value is a Figma Variable ID
 *   (e.g. "VariableID:123:456"). Omit a key to skip binding that property.
 * @returns {Promise<{ mutatedNodeIds: string[] }>}
 *   List of node IDs that were mutated (for audit/validation purposes).
 */
async function bindVariablesToComponent(component, bindings) {
  const mutatedNodeIds = []

  if (!component) {
    return { mutatedNodeIds }
  }

  // --- Fills ---
  if (bindings.fills) {
    const fillVar = await figma.variables.getVariableByIdAsync(bindings.fills)
    if (fillVar) {
      const existingFills = component.fills
      if (Array.isArray(existingFills) && existingFills.length > 0) {
        // Bind the color of the first fill to the variable
        const boundFill = figma.variables.setBoundVariableForPaint(
          existingFills[0],
          'color',
          fillVar,
        )
        component.fills = [boundFill, ...existingFills.slice(1)]
      } else {
        // No existing fill — create a solid fill bound to the variable
        const boundFill = figma.variables.setBoundVariableForPaint(
          { type: 'SOLID', color: { r: 0.5, g: 0.5, b: 0.5 } },
          'color',
          fillVar,
        )
        component.fills = [boundFill]
      }
      mutatedNodeIds.push(component.id)
    }
  }

  // --- Strokes ---
  if (bindings.strokes) {
    const strokeVar = await figma.variables.getVariableByIdAsync(bindings.strokes)
    if (strokeVar) {
      const existingStrokes = component.strokes
      if (Array.isArray(existingStrokes) && existingStrokes.length > 0) {
        const boundStroke = figma.variables.setBoundVariableForPaint(
          existingStrokes[0],
          'color',
          strokeVar,
        )
        component.strokes = [boundStroke, ...existingStrokes.slice(1)]
      } else {
        const boundStroke = figma.variables.setBoundVariableForPaint(
          { type: 'SOLID', color: { r: 0.5, g: 0.5, b: 0.5 } },
          'color',
          strokeVar,
        )
        component.strokes = [boundStroke]
      }
      if (!mutatedNodeIds.includes(component.id)) {
        mutatedNodeIds.push(component.id)
      }
    }
  }

  // --- Spacing properties (FLOAT variables bound via setBoundVariable) ---
  const floatBindings = [
    ['paddingTop', 'paddingTop'],
    ['paddingBottom', 'paddingBottom'],
    ['paddingLeft', 'paddingLeft'],
    ['paddingRight', 'paddingRight'],
    ['itemSpacing', 'itemSpacing'],
    ['cornerRadius', 'cornerRadius'],
  ]

  for (const [bindingKey, figmaProp] of floatBindings) {
    if (bindings[bindingKey]) {
      const variable = await figma.variables.getVariableByIdAsync(bindings[bindingKey])
      if (variable) {
        component.setBoundVariable(figmaProp, variable)
        if (!mutatedNodeIds.includes(component.id)) {
          mutatedNodeIds.push(component.id)
        }
      }
    }
  }

  return { mutatedNodeIds }
}
