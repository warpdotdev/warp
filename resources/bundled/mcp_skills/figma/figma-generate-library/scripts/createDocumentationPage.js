/**
 * createDocumentationPage
 *
 * Creates a new Figma page with a standardized documentation layout: a page
 * title, optional description, and an ordered list of sections each built by
 * a caller-supplied `contentFn`. The content function receives the section
 * frame and may append any nodes to it.
 *
 * This function is used for standalone documentation pages (e.g. a Foundations
 * page, a Getting Started page, or a component page with documentation).
 * It does not handle component sets — those live on separate pages created by
 * createComponentWithVariants.
 *
 * @param {string} pageName - The Figma page name (e.g. "Foundations", "Getting Started").
 * @param {{
 *   title: string,
 *   description?: string,
 *   sections: Array<{
 *     name: string,
 *     contentFn: (sectionFrame: FrameNode) => Promise<void>
 *   }>
 * }} config
 *   - `title`: Large heading displayed at the top of the page.
 *   - `description`: Optional subtitle displayed below the heading.
 *   - `sections`: Ordered list of sections. Each section gets its own frame
 *     with a heading and is passed to `contentFn` for population.
 * @param {string} [runId] - Optional dsb_run_id to tag every created node.
 * @returns {Promise<{
 *   page: PageNode,
 *   titleNode: TextNode,
 *   frameIds: string[]
 * }>}
 *   `frameIds` is an ordered list of IDs for the root frame and each section frame.
 */
async function createDocumentationPage(pageName, config, runId) {
  await figma.loadFontAsync({ family: 'Inter', style: 'Bold' })
  await figma.loadFontAsync({ family: 'Inter', style: 'Regular' })
  await figma.loadFontAsync({ family: 'Inter', style: 'Medium' })

  // Create and activate the page
  const page = figma.createPage()
  page.name = pageName
  await figma.setCurrentPageAsync(page)

  if (runId) {
    page.setPluginData('dsb_run_id', runId)
    page.setPluginData('dsb_key', `page/${pageName}`)
  }

  const frameIds = []

  // Root scroll container — 1440px wide, auto-height
  const root = figma.createFrame()
  root.name = pageName
  root.layoutMode = 'VERTICAL'
  root.primaryAxisAlignItems = 'MIN'
  root.counterAxisAlignItems = 'MIN'
  root.itemSpacing = 80
  root.paddingTop = 80
  root.paddingBottom = 120
  root.paddingLeft = 80
  root.paddingRight = 80
  root.layoutSizingHorizontal = 'FIXED'
  root.layoutSizingVertical = 'HUG'
  root.resize(1440, 1)
  root.fills = [{ type: 'SOLID', color: { r: 1, g: 1, b: 1 } }]
  root.x = 0
  root.y = 0
  page.appendChild(root)

  if (runId) {
    root.setPluginData('dsb_run_id', runId)
    root.setPluginData('dsb_key', `frame/root/${pageName}`)
  }

  frameIds.push(root.id)

  // Page header: title + optional description
  const header = figma.createFrame()
  header.name = 'Header'
  header.layoutMode = 'VERTICAL'
  header.itemSpacing = 12
  header.layoutSizingHorizontal = 'FILL'
  header.layoutSizingVertical = 'HUG'
  header.fills = []
  root.appendChild(header)

  const titleNode = figma.createText()
  titleNode.fontName = { family: 'Inter', style: 'Bold' }
  titleNode.characters = config.title
  titleNode.fontSize = 40
  titleNode.fills = [{ type: 'SOLID', color: { r: 0.07, g: 0.07, b: 0.07 } }]
  titleNode.layoutSizingHorizontal = 'FILL'
  header.appendChild(titleNode)

  if (config.description) {
    const descNode = figma.createText()
    descNode.fontName = { family: 'Inter', style: 'Regular' }
    descNode.characters = config.description
    descNode.fontSize = 16
    descNode.lineHeight = { value: 24, unit: 'PIXELS' }
    descNode.fills = [{ type: 'SOLID', color: { r: 0.4, g: 0.4, b: 0.4 } }]
    descNode.layoutSizingHorizontal = 'FILL'
    header.appendChild(descNode)
  }

  // Sections
  for (const section of config.sections) {
    const sectionFrame = figma.createFrame()
    sectionFrame.name = `Section/${section.name}`
    sectionFrame.layoutMode = 'VERTICAL'
    sectionFrame.itemSpacing = 20
    sectionFrame.layoutSizingHorizontal = 'FILL'
    sectionFrame.layoutSizingVertical = 'HUG'
    sectionFrame.fills = []
    root.appendChild(sectionFrame)

    if (runId) {
      sectionFrame.setPluginData('dsb_run_id', runId)
      sectionFrame.setPluginData('dsb_key', `frame/section/${pageName}/${section.name}`)
    }

    // Section heading
    const sectionHeading = figma.createText()
    sectionHeading.fontName = { family: 'Inter', style: 'Bold' }
    sectionHeading.characters = section.name
    sectionHeading.fontSize = 24
    sectionHeading.fills = [{ type: 'SOLID', color: { r: 0.07, g: 0.07, b: 0.07 } }]
    sectionHeading.layoutSizingHorizontal = 'FILL'
    sectionFrame.appendChild(sectionHeading)

    // Invoke the caller's content function to populate the section
    await section.contentFn(sectionFrame)

    frameIds.push(sectionFrame.id)
  }

  return { page, titleNode, frameIds }
}
