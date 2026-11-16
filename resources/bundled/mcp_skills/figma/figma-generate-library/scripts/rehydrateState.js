/**
 * Scans the entire Figma file for nodes tagged with dsb_* pluginData
 * and returns a complete state map for session recovery.
 *
 * Use this at the start of every new session, after context truncation,
 * or when resuming an interrupted build.
 *
 * @param {string} runId - The run ID to filter by (optional — if omitted, returns ALL dsb-tagged nodes)
 * @returns {{ runId: string, taggedNodes: Object<string, {nodeId: string, type: string, name: string, phase: string}>, variableCollections: Array, variables: Array, styles: Array }}
 */
async function rehydrateState(runId) {
  const taggedNodes = {}
  const variableCollections = []
  const variables = []
  const styles = []

  // Scan all pages for dsb-tagged scene nodes
  for (const page of figma.root.children) {
    await figma.setCurrentPageAsync(page)

    // Check the page itself
    const pageRunId = page.getPluginData('dsb_run_id')
    const pageKey = page.getPluginData('dsb_key')
    if (pageKey && (!runId || pageRunId === runId)) {
      taggedNodes[pageKey] = {
        nodeId: page.id,
        type: page.type,
        name: page.name,
        phase: page.getPluginData('dsb_phase') || 'unknown',
      }
    }

    // Scan all descendants
    page.findAll((node) => {
      const nodeRunId = node.getPluginData('dsb_run_id')
      const nodeKey = node.getPluginData('dsb_key')
      if (nodeKey && (!runId || nodeRunId === runId)) {
        taggedNodes[nodeKey] = {
          nodeId: node.id,
          type: node.type,
          name: node.name,
          phase: node.getPluginData('dsb_phase') || 'unknown',
        }
      }
      return false // don't collect, just scan
    })
  }

  // Inventory variable collections (variables don't support pluginData — use name-based lookup)
  const collections = await figma.variables.getLocalVariableCollectionsAsync()
  for (const coll of collections) {
    variableCollections.push({
      id: coll.id,
      name: coll.name,
      modes: coll.modes.map((m) => ({ modeId: m.modeId, name: m.name })),
      variableCount: coll.variableIds.length,
    })
  }

  // Inventory variables (name + collection for idempotency key)
  const allVars = await figma.variables.getLocalVariablesAsync()
  for (const v of allVars) {
    variables.push({
      id: v.id,
      name: v.name,
      collectionId: v.variableCollectionId,
      resolvedType: v.resolvedType,
    })
  }

  // Inventory styles
  for (const s of figma.getLocalTextStyles()) {
    styles.push({ id: s.id, name: s.name, type: 'TEXT' })
  }
  for (const s of figma.getLocalEffectStyles()) {
    styles.push({ id: s.id, name: s.name, type: 'EFFECT' })
  }
  for (const s of figma.getLocalPaintStyles()) {
    styles.push({ id: s.id, name: s.name, type: 'PAINT' })
  }

  return {
    runId: runId || 'all',
    taggedNodes,
    taggedNodeCount: Object.keys(taggedNodes).length,
    variableCollections,
    variableCount: variables.length,
    variables,
    styleCount: styles.length,
    styles,
  }
}
