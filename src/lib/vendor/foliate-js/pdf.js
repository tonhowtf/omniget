// @ts-nocheck
const pdfjsPath = path => `/vendor/pdfjs/${path}`

let pdfjsLib
let pdfjsLibPromise
const loadPDFJS = async () => {
    if (!pdfjsLibPromise) {
        pdfjsLibPromise = import('pdfjs-dist/legacy/build/pdf.min.mjs').then(() => {
            pdfjsLib = globalThis.pdfjsLib
            const workerSrc = pdfjsLib.GlobalWorkerOptions.workerSrc
            if (!workerSrc || workerSrc === './pdf.worker.mjs') {
                pdfjsLib.GlobalWorkerOptions.workerSrc = pdfjsPath('pdf.worker.min.mjs')
            }
            return pdfjsLib
        })
    }
    return pdfjsLibPromise
}

const DC_NS = 'http://purl.org/dc/elements/1.1/'
const RDF_NS = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#'
const CALIBRE_NS = 'http://calibre-ebook.com/xmp-namespace'
const CALIBRE_SI_NS = 'http://calibre-ebook.com/xmp-namespace-series-index'

/** @typedef {string | number[] | Uint8Array | null | undefined} PDFMetadataValue */
/** @typedef {Record<string, PDFMetadataValue>} PDFInfo */

// PDF 32000-1, Table D.2. This mirrors pdf.js's stringToPDFString so Info
// dictionary values read natively have the same Unicode representation as
// values returned by PDFDocumentProxy.getMetadata().
const PDF_STRING_TRANSLATE_TABLE = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0x2d8, 0x2c7, 0x2c6, 0x2d9, 0x2dd, 0x2db, 0x2da, 0x2dc,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0x2022, 0x2020, 0x2021, 0x2026, 0x2014, 0x2013, 0x192, 0x2044,
    0x2039, 0x203a, 0x2212, 0x2030, 0x201e, 0x201c, 0x201d, 0x2018,
    0x2019, 0x201a, 0x2122, 0xfb01, 0xfb02, 0x141, 0x152, 0x160,
    0x178, 0x17d, 0x131, 0x142, 0x153, 0x161, 0x17e, 0, 0x20ac,
]

/** @param {number[] | Uint8Array} value */
const toUint8Array = value => value instanceof Uint8Array ? value : new Uint8Array(value)

const removeEscapeSequences = value => {
    let result = ''
    let escaping = false
    for (const char of value) {
        if (char.charCodeAt(0) === 0x1b) {
            escaping = !escaping
        } else if (!escaping) {
            result += char
        }
    }
    return result
}

/** @param {PDFMetadataValue} value */
export const decodePDFString = value => {
    if (value == null || typeof value === 'string') return value
    let bytes = toUint8Array(value)
    let encoding
    if (bytes[0] === 0xfe && bytes[1] === 0xff) encoding = 'utf-16be'
    else if (bytes[0] === 0xff && bytes[1] === 0xfe) encoding = 'utf-16le'
    else if (bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) encoding = 'utf-8'
    if (encoding) {
        if (encoding.startsWith('utf-16') && bytes.length % 2 === 1) bytes = bytes.slice(0, -1)
        try {
            return removeEscapeSequences(new TextDecoder(encoding, { fatal: true }).decode(bytes))
        } catch {
            // Match pdf.js: fall through to PDFDocEncoding when BOM decoding fails.
        }
    }
    const chars = []
    for (let i = 0; i < bytes.length; i++) {
        const byte = bytes[i]
        if (byte === 0x1b) {
            while (++i < bytes.length && bytes[i] !== 0x1b) {
                // Skip the escape sequence.
            }
            continue
        }
        const code = PDF_STRING_TRANSLATE_TABLE[byte]
        chars.push(String.fromCharCode(code || byte))
    }
    return chars.join('')
}

/** @param {PDFMetadataValue} value */
const decodeXMP = value => {
    if (!value) return null
    const raw = typeof value === 'string'
        ? value
        : new TextDecoder('utf-8').decode(toUint8Array(value))
    return raw.replace(/^[^<]+/, '').replaceAll(/>\\376\\377([^<]+)/g, (all, codes) => {
        const bytes = codes.replaceAll(/\\([0-3])([0-7])([0-7])/g,
            (code, d1, d2, d3) => String.fromCharCode(d1 * 64 + d2 * 8 + d3))
            .replaceAll(/&(amp|apos|gt|lt|quot);/g, (str, name) => ({
                amp: '&', apos: "'", gt: '>', lt: '<', quot: '"',
            })[name])
        const chars = ['>']
        for (let i = 0; i < bytes.length; i += 2) {
            const code = bytes.charCodeAt(i) * 256 + bytes.charCodeAt(i + 1)
            chars.push(code >= 32 && code < 127 && code !== 60 && code !== 62 && code !== 38
                ? String.fromCharCode(code)
                : `&#x${(0x10000 + code).toString(16).substring(1)};`)
        }
        return chars.join('')
    })
}

const parseXMP = raw => {
    if (!raw) return { doc: null, values: new Map() }
    let doc
    try {
        doc = new DOMParser().parseFromString(raw, 'application/xml')
    } catch {
        return { doc: null, values: new Map() }
    }
    if (doc.getElementsByTagName('parsererror').length) {
        return { doc: null, values: new Map() }
    }
    const values = new Map()
    for (const description of doc.getElementsByTagNameNS(RDF_NS, 'Description')) {
        for (const entry of description.children) {
            if (entry.namespaceURI !== DC_NS) continue
            const name = `dc:${entry.localName.toLowerCase()}`
            if (name === 'dc:creator' || name === 'dc:subject') {
                const container = entry.firstElementChild
                const items = container
                    ? Array.from(container.children)
                        .filter(item => item.namespaceURI === RDF_NS && item.localName === 'li')
                        .map(item => item.textContent.trim())
                    : []
                values.set(name, items)
            } else {
                values.set(name, entry.textContent.trim())
            }
        }
    }
    return { doc, values }
}

const getInfo = (info, name) =>
    decodePDFString(info?.[name] ?? info?.[name.toLowerCase()]) ?? undefined

const getCalibreSeries = doc => {
    const series = doc?.getElementsByTagNameNS(CALIBRE_NS, 'series').item(0)
    const name = series?.getElementsByTagNameNS(RDF_NS, 'value').item(0)?.textContent?.trim()
    if (!name) return null
    const position = series.getElementsByTagNameNS(CALIBRE_SI_NS, 'series_index')
        .item(0)?.textContent?.trim()
    return position ? { name, position } : { name }
}

/**
 * Normalize PDF Info/XMP into the metadata shape used by the full reader.
 * @param {{ info?: PDFInfo, xmp?: PDFMetadataValue }} [input]
 */
export const parsePDFMetadata = ({ info = {}, xmp = null } = {}) => {
    const raw = decodeXMP(xmp)
    const { doc, values } = parseXMP(raw)
    const metadata = {
        title: values.get('dc:title') ?? getInfo(info, 'Title'),
        author: values.get('dc:creator') ?? getInfo(info, 'Author'),
        contributor: values.get('dc:contributor'),
        description: values.get('dc:description') ?? getInfo(info, 'Subject'),
        language: values.get('dc:language'),
        publisher: values.get('dc:publisher'),
        subject: values.get('dc:subject'),
        identifier: values.get('dc:identifier'),
        source: values.get('dc:source'),
        rights: values.get('dc:rights'),
    }
    const series = getCalibreSeries(doc)
    if (series) metadata.belongsTo = { series }
    return metadata
}

const fetchText = async url => await (await fetch(url)).text()

// The OS accessibility "font size" setting scales every piece of WebView-rendered
// text (including this transparent selection/highlight text layer) but leaves the
// page's canvas bitmap untouched. Only the glyph *size* (a font-size) is scaled;
// the text layer's positions are percentages of the `--total-scale-factor`-sized
// container and are not. Left uncorrected the glyphs render `fontScale`x larger
// than the ones baked into the canvas, so selection and highlight rectangles
// overshoot the text into the blank margins and sit too low (readest #4480).
// Measure the scale here so render() can divide it back out of the glyph-size
// lever only. offsetHeight of a 100px/line-height-1 box reflects the OS font
// scaling but not devicePixelRatio or CSS transforms, so it isolates it.
const getFontScale = doc => {
    const probe = doc.createElement('div')
    probe.style.cssText = 'position:absolute;left:-9999px;top:0;visibility:hidden;'
        + 'font-size:100px;line-height:1;text-size-adjust:none;-webkit-text-size-adjust:none'
    probe.textContent = 'x'
    doc.body.append(probe)
    const fontScale = probe.offsetHeight / 100
    probe.remove()
    return fontScale > 0 ? fontScale : 1
}

let textLayerBuilderCSS = null
let annotationLayerBuilderCSS = null

// Track active render tasks per iframe document to cancel superseded renders
const activeRenderTasks = new WeakMap()
// Generation counter per document to detect stale renders after async gaps
const renderGenerations = new WeakMap()
// What each document was last rendered for, so an identical re-render can be
// skipped instead of rebuilding the text layer (see `render`)
const renderedFor = new WeakMap()

// Set up panning and selection event handlers once per iframe document
export const setupPanningEvents = (doc) => {
    if (doc._readestEventsInitialized) return
    doc._readestEventsInitialized = true

    const container = doc.querySelector('.textLayer')
    if (!container) return

    let isPanning = false
    let startX = 0
    let startY = 0
    let scrollLeft = 0
    let scrollTop = 0
    let scrollParent = null

    const findScrollableParent = (element) => {
        let current = element
        while (current) {
            if (current !== document.body && current.nodeType === 1) {
                const style = window.getComputedStyle(current)
                const overflow = style.overflow + style.overflowY + style.overflowX
                if (/(auto|scroll)/.test(overflow)) {
                    if (current.scrollHeight > current.clientHeight ||
                        current.scrollWidth > current.clientWidth) {
                        return current
                    }
                }
            }
            if (current.parentElement) {
                current = current.parentElement
            } else if (current.parentNode && current.parentNode.host) {
                current = current.parentNode.host
            } else {
                break
            }
        }
        return window
    }

    container.onpointerdown = (e) => {
        const selection = doc.getSelection()
        const hasTextSelection = selection && selection.toString().length > 0

        const elementUnderCursor = doc.elementFromPoint(e.clientX, e.clientY)
        const hasTextUnderneath = elementUnderCursor &&
                             (elementUnderCursor.tagName === 'SPAN' || elementUnderCursor.tagName === 'P') &&
                             elementUnderCursor.textContent.trim().length > 0

        if (!hasTextUnderneath && !hasTextSelection) {
            isPanning = true
            startX = e.screenX
            startY = e.screenY

            const iframe = doc.defaultView?.frameElement
            if (iframe) {
                scrollParent = findScrollableParent(iframe)
                if (scrollParent === window) {
                    scrollLeft = window.scrollX || window.pageXOffset
                    scrollTop = window.scrollY || window.pageYOffset
                } else {
                    scrollLeft = scrollParent.scrollLeft
                    scrollTop = scrollParent.scrollTop
                }
                container.style.cursor = 'grabbing'
            }
        } else {
            container.classList.add('selecting')
        }
    }

    container.onpointermove = (e) => {
        if (isPanning && scrollParent) {
            e.preventDefault()

            const dx = e.screenX - startX
            const dy = e.screenY - startY
            // Panning a zoomed page is a script write, not native scrolling, so
            // `touch-action` cannot constrain it. The renderer mirrors the
            // horizontal pan lock onto this document's root element (#5976);
            // read it back so a drag that is only slightly diagonal can't shift
            // the page sideways again.
            const lockX = doc.documentElement.style.touchAction === 'pan-y'

            if (scrollParent === window) {
                window.scrollTo(lockX ? window.scrollX : scrollLeft - dx, scrollTop - dy)
            } else {
                if (!lockX) scrollParent.scrollLeft = scrollLeft - dx
                scrollParent.scrollTop = scrollTop - dy
            }
        }
    }

    container.onpointerup = () => {
        if (isPanning) {
            isPanning = false
            scrollParent = null
            container.style.cursor = 'grab'
        } else {
            container.classList.remove('selecting')
        }
    }

    container.onpointerleave = () => {
        if (isPanning) {
            isPanning = false
            scrollParent = null
            container.style.cursor = 'grab'
        }
    }

    doc.addEventListener('selectionchange', () => {
        const selection = doc.getSelection()
        if (selection && selection.toString().length > 0) {
            container.style.cursor = 'text'
        } else if (!isPanning) {
            container.style.cursor = 'grab'
        }
    })

    container.style.cursor = 'grab'
}

// iOS kills the WKWebView content process when it exceeds a per-process memory
// high-water limit (~2 GB). A device crash log for readest #5118 shows the
// foreground WebContent process reaching 2.1 GB while paging a PDF, right before
// the reader "closed". Both a page's canvas bitmap and its WebKit backing layer
// are allocated at the render scale, so their memory grows with the SQUARE of the
// device pixel ratio. Phones report dpr 3, which is the tipping factor.
// Rendering at 2x instead of 3x is still retina-sharp but uses ~2.25x less memory
// per page (the crisp, selectable text layer is a separate DOM layer, unaffected).
const MAX_RENDER_DPR = 2
// Hard ceiling on a single page's bitmap area (~3.1 Mpx ≈ 12.6 MB) so a large
// tablet page can't blow the budget even after the dpr clamp.
const MAX_CANVAS_PIXELS = 2048 * 1536

// Only mobile WebViews get that budget. Desktop browsers have no per-process
// memory ceiling, and a page fitted to a desktop window is several times the
// budget on its own, so clamping there bought nothing and cost sharpness: the
// raster ended up coarser than the screen, the browser upscaled it into the CSS
// box, and PDF text looked blurry (readest #5251). iPadOS reports a desktop
// ("Macintosh") user agent, so touch points are what give a tablet away.
const isMobileWebView = () => {
    const ua = navigator.userAgent
    return /Android|iPhone|iPad|iPod/i.test(ua)
        || (/Macintosh/i.test(ua) && navigator.maxTouchPoints > 1)
}

// The device pixel ratio to rasterise this page at: the real dpr on desktop, or
// on mobile the dpr clamped by both MAX_RENDER_DPR and the per-canvas pixel
// budget. Never below 1 (CSS resolution).
const getRenderDpr = (page, zoom) => {
    let dpr = devicePixelRatio || 1
    if (isMobileWebView()) {
        dpr = Math.min(dpr, MAX_RENDER_DPR)
        const { width, height } = page.getViewport({ scale: zoom || 1 })
        const area = width * height * dpr * dpr
        if (area > MAX_CANVAS_PIXELS) dpr *= Math.sqrt(MAX_CANVAS_PIXELS / area)
    }
    return Math.max(1, dpr)
}

const render = async (page, doc, zoom, pageColors) => {
    if (!doc) return

    // Rendering tears the text layer down and builds it again, which detaches
    // every Range held into it -- the sentence ranges TTS is reading from, and
    // anything anchored for selection or highlighting. fixed-layout re-renders
    // from a ResizeObserver on any layout change, including the one a page turn
    // itself causes, so the same page gets rendered twice per turn and the
    // second pass kills the ranges TTS just built (readest #6071). Skip the work
    // when nothing that affects the output has changed.
    const signature = [zoom, pageColors?.background, pageColors?.foreground,
        getFontScale(doc)].join('|')
    const rendered = renderedFor.get(doc)
    if (rendered?.page === page && rendered.signature === signature) return

    // Increment generation to invalidate any in-progress render for this doc
    const generation = (renderGenerations.get(doc) || 0) + 1
    renderGenerations.set(doc, generation)
    renderedFor.set(doc, { page, signature, generation })
    // Let a later render retry after this one bails without replacing the DOM.
    // A newer render overwrites the entry with its own generation, so only the
    // render that still owns it clears it.
    const forget = () => {
        if (renderedFor.get(doc)?.generation === generation) renderedFor.delete(doc)
    }

    // Cancel any in-progress render task for this document
    const existingTask = activeRenderTasks.get(doc)
    if (existingTask) {
        existingTask.cancel()
        activeRenderTasks.delete(doc)
    }

    // Rasterise the page bitmap over-sampled (clamped for the iOS content-process
    // memory budget, see getRenderDpr / readest #5118) but lay the whole DOM out
    // at the true display size. The <canvas> element natively downscales its
    // bitmap to its CSS box, so the raster stays crisp WITHOUT scaling the
    // document. Scaling the document with `transform` promotes the whole page to
    // one over-sized GPU IOSurface that OOM-kills the iOS WebContent process on
    // zoom; scaling it with `zoom` throws off getBoundingClientRect, misplacing
    // text selection and the annotation toolbar. Neither is used: the text and
    // annotation layers live in real display coordinates.
    const renderDpr = getRenderDpr(page, zoom)
    const renderScale = zoom * renderDpr
    doc.documentElement.style.setProperty('--total-scale-factor', zoom)
    doc.documentElement.style.setProperty('--user-unit', '1')
    doc.documentElement.style.setProperty('--scale-round-x', '1px')
    doc.documentElement.style.setProperty('--scale-round-y', '1px')
    // The bitmap viewport is over-sampled; the display viewport drives the CSS
    // box, the text layer and the annotation layer (all in display coordinates).
    const renderViewport = page.getViewport({ scale: renderScale })
    const displayViewport = page.getViewport({ scale: zoom })

    // the canvas must be in the `PDFDocument`'s `ownerDocument`
    // (`globalThis.document` by default); that's where the fonts are loaded
    const canvas = document.createElement('canvas')
    canvas.height = renderViewport.height
    canvas.width = renderViewport.width
    // The CSS box is the un-truncated display size, so the (integer-truncated)
    // over-sampled bitmap is scaled by the browser to fill the page box exactly.
    // Pinning the box to the display viewport (rather than letting the truncated
    // bitmap drive layout) also keeps the left page flush to the spine of a
    // two-page spread instead of exposing a one-pixel white seam (#4587).
    canvas.style.width = `${displayViewport.width}px`
    canvas.style.height = `${displayViewport.height}px`
    const canvasContext = canvas.getContext('2d')
    const renderTask = page.render({ canvasContext, viewport: renderViewport, pageColors })
    activeRenderTasks.set(doc, renderTask)

    try {
        await renderTask.promise
    } catch {
        // Render was cancelled or failed — release canvas bitmap memory
        canvas.width = 0
        canvas.height = 0
        forget()
        return
    } finally {
        if (activeRenderTasks.get(doc) === renderTask) {
            activeRenderTasks.delete(doc)
        }
    }

    // Bail out if a newer render has started or iframe was removed
    if (renderGenerations.get(doc) !== generation || !doc.defaultView) {
        canvas.width = 0
        canvas.height = 0
        forget()
        return
    }

    const canvasElement = doc.querySelector('#canvas')
    if (!canvasElement) {
        canvas.width = 0
        canvas.height = 0
        forget()
        return
    }

    // Release old canvas bitmap memory before replacing
    const oldCanvas = canvasElement.querySelector('canvas')
    if (oldCanvas) {
        oldCanvas.width = 0
        oldCanvas.height = 0
    }
    canvasElement.replaceChildren(doc.adoptNode(canvas))

    // Clear text layer before re-rendering to prevent DOM accumulation
    const container = doc.querySelector('.textLayer')
    container.replaceChildren()
    const textLayer = new pdfjsLib.TextLayer({
        textContentSource: await page.streamTextContent(),
        container, viewport: displayViewport,
    })
    await textLayer.render()

    // Bail out if superseded after async text layer render
    if (renderGenerations.get(doc) !== generation) {
        forget()
        return
    }

    // Counteract the OS font-size accessibility scaling on the text layer's glyph
    // size only (see getFontScale). `--text-scale-factor` feeds `font-size` and
    // nothing else, so dividing it leaves positions (which scale with
    // `--total-scale-factor`) aligned with the canvas at any font-size setting.
    const fontScale = getFontScale(doc)
    if (fontScale !== 1) container.style.setProperty('--text-scale-factor',
        `calc(var(--total-scale-factor) * var(--min-font-size) / ${fontScale})`)

    // hide "offscreen" canvases appended to document when rendering text layer
    // https://github.com/mozilla/pdf.js/blob/642b9a5ae67ef642b9a8808fd9efd447e8c350e2/web/pdf_viewer.css#L51-L58
    for (const hiddenCanvas of document.querySelectorAll('.hiddenCanvasElement'))
        Object.assign(hiddenCanvas.style, {
            position: 'absolute',
            top: '0',
            left: '0',
            width: '0',
            height: '0',
            display: 'none',
        })

    // fix text selection
    // https://github.com/mozilla/pdf.js/blob/642b9a5ae67ef642b9a8808fd9efd447e8c350e2/web/text_layer_builder.js#L105-L107
    const endOfContent = document.createElement('div')
    endOfContent.className = 'endOfContent'
    container.append(endOfContent)

    // Set up panning/selection event handlers once per document
    setupPanningEvents(doc)

    // Clear annotation layer before re-rendering to prevent DOM accumulation
    const div = doc.querySelector('.annotationLayer')
    div.replaceChildren()
    const linkService = {
        goToDestination: () => {},
        getDestinationHash: dest => JSON.stringify(dest),
        // pdf.js AnnotationLayer calls getAnchorUrl for named-action / GoTo link
        // annotations; without it the render rejects with "getAnchorUrl is not a
        // function" (READEST-2M). Match pdf.js SimpleLinkService, which returns ''.
        getAnchorUrl: () => '',
        addLinkAttributes: (link, url) => link.href = url,
    }
    await new pdfjsLib.AnnotationLayer({ page, viewport: displayViewport, div, linkService }).render({
        annotations: await page.getAnnotations(),
    })
}

const renderPage = async (page, getImageBlob) => {
    const viewport = page.getViewport({ scale: 1 })
    if (getImageBlob) {
        const canvas = document.createElement('canvas')
        canvas.height = viewport.height
        canvas.width = viewport.width
        const canvasContext = canvas.getContext('2d')
        await page.render({ canvasContext, viewport }).promise
        return new Promise(resolve => canvas.toBlob(blob => {
            // Release canvas bitmap memory after extracting the blob
            canvas.width = 0
            canvas.height = 0
            resolve(blob)
        }))
    }
    // https://github.com/mozilla/pdf.js/blob/642b9a5ae67ef642b9a8808fd9efd447e8c350e2/web/text_layer_builder.css
    if (textLayerBuilderCSS == null) {
        textLayerBuilderCSS = await fetchText(pdfjsPath('text_layer_builder.css'))
    }
    // https://github.com/mozilla/pdf.js/blob/642b9a5ae67ef642b9a8808fd9efd447e8c350e2/web/annotation_layer_builder.css
    if (annotationLayerBuilderCSS == null) {
        annotationLayerBuilderCSS = await fetchText(pdfjsPath('annotation_layer_builder.css'))
    }
    const data = `
        <!DOCTYPE html>
        <html lang="en">
        <meta charset="utf-8">
        <meta name="viewport" content="width=${viewport.width}, height=${viewport.height}">
        <style>
        html, body {
            margin: 0;
            padding: 0;
        }
        ${textLayerBuilderCSS}
        ${annotationLayerBuilderCSS}
        </style>
        <div id="canvas"></div>
        <div class="textLayer"></div>
        <div class="annotationLayer"></div>
    `
    const src = URL.createObjectURL(new Blob([data], { type: 'text/html' }))
    const onZoom = ({ doc, scale, pageColors }) => render(page, doc, scale, pageColors)
    return { src, data, onZoom }
}

const makeTOCItem = async (item, pdf) => {
    let pageIndex = undefined

    if (item.dest) {
        try {
            const dest = typeof item.dest === 'string'
                ? await pdf.getDestination(item.dest)
                : item.dest
            if (dest?.[0]) {
                pageIndex = await pdf.getPageIndex(dest[0])
            }
        } catch (e) {
            console.warn('Failed to get page index for TOC item:', item.title, e)
        }
    }

    return {
        label: item.title,
        href: item.dest ? JSON.stringify(item.dest) : '',
        index: pageIndex,
        subitems: item.items?.length
            ? await Promise.all(item.items.map(i => makeTOCItem(i, pdf)))
            : null,
    }
}

// Cache of decoded pdf.js page objects and their rendered HTML blobs. These are
// cheap (page metadata + a small blob URL, not the large canvas bitmap, which
// lives in the visible iframe) so this can comfortably exceed the live-canvas
// cap in fixed-layout's scroll mode, sparing a re-parse when the reader scrolls
// back over a recently seen page.
const MAX_CACHED_PAGES = 16

// Maximum number of range reads to keep in flight at once. While parsing a
// large PDF's cross-reference and object streams, pdf.js can request hundreds
// of byte ranges in a single burst. A real HTTP transport is implicitly
// throttled by the browser's per-host connection limit (~6); the custom file
// schemes readest serves these reads through (Android's `rangefile` /
// `shouldInterceptRequest`, iOS' native file bridge) have no such limit, so
// firing every request at once floods the native handler and exhausts the
// WebView's heap, crashing on 50 MB+ PDFs (readest #3470). Throttle here.
const MAX_CONCURRENT_RANGES = 6

export const makePDF = async file => {
    await loadPDFJS()
    const transport = new pdfjsLib.PDFDataRangeTransport(file.size, [])
    // Bound the concurrent range reads instead of dispatching them all at once.
    let active = 0
    const queue = []
    const pump = () => {
        while (active < MAX_CONCURRENT_RANGES && queue.length) {
            const [begin, end] = queue.shift()
            active++
            file.slice(begin, end).arrayBuffer()
                .then(chunk => transport.onDataRange(begin, chunk))
                .finally(() => { active--; pump() })
        }
    }
    transport.requestDataRange = (begin, end) => {
        queue.push([begin, end])
        pump()
    }
    const loadingTask = pdfjsLib.getDocument({
        range: transport,
        wasmUrl: pdfjsPath(''),
        cMapUrl: pdfjsPath('cmaps/'),
        standardFontDataUrl: pdfjsPath('standard_fonts/'),
        isEvalSupported: false,
    })
    const pdf = await loadingTask.promise

    // Get viewport dimensions from first page for fixed-layout rendering
    const firstPage = await pdf.getPage(1)
    const firstViewport = firstPage.getViewport({ scale: 1 })
    const book = { rendition: {
        layout: 'pre-paginated',
        viewport: { width: firstViewport.width, height: firstViewport.height },
    } }

    const { metadata, info } = await pdf.getMetadata() ?? {}
    book.metadata = parsePDFMetadata({ info, xmp: metadata?.getRaw?.() })

    // PDFs bound right-to-left (Japanese photo books, manga) declare it in the
    // catalog's ViewerPreferences; surface it as book.dir so the fixed-layout
    // renderer pairs and orders two-page spreads right-to-left.
    const viewerPreferences = await pdf.getViewerPreferences().catch(() => null)
    const direction = viewerPreferences?.get?.('Direction')
        ?? viewerPreferences?.Direction
    if (direction === 'R2L') book.dir = 'rtl'

    const outline = await pdf.getOutline()
    book.toc = outline ? await Promise.all(outline.map(item => makeTOCItem(item, pdf))) : null

    // Page labels (PDF 32000-1 §12.4.2) are the numbers printed on the pages
    // -- roman-numeral front matter, a body that restarts at 1 -- and are what
    // the book's own TOC means by "page 139", as opposed to the physical index
    // into the file. Expose them as the page list so they reach readers through
    // the same `pageItem` channel as an EPUB page-list nav. Like PDF.js, ignore
    // labels that merely restate the physical page numbers or are all empty.
    const labels = await pdf.getPageLabels().catch(() => null)
    book.pageList = labels?.some((label, i) => label && label !== String(i + 1))
        ? labels.map((label, i) => ({ label, href: JSON.stringify(i), index: i }))
        : null

    const cache = new Map()
    const pageCache = new Map()
    const getPage = async (i) => {
        const cached = pageCache.get(i)
        if (cached) {
            // Move to end for LRU ordering
            pageCache.delete(i)
            pageCache.set(i, cached)
            return cached
        }
        const page = await pdf.getPage(i + 1)
        pageCache.set(i, page)

        // Evict oldest pages when over limit, freeing internal page data
        while (pageCache.size > MAX_CACHED_PAGES) {
            const oldestKey = pageCache.keys().next().value
            const oldPage = pageCache.get(oldestKey)
            pageCache.delete(oldestKey)
            oldPage?.cleanup()
        }

        return page
    }
    book.sections = Array.from({ length: pdf.numPages }).map((_, i) => ({
        id: i,
        load: async () => {
            const cached = cache.get(i)
            if (cached) {
                // Move to end for LRU ordering
                cache.delete(i)
                cache.set(i, cached)
                return cached
            }
            const url = await renderPage(await getPage(i))
            cache.set(i, url)

            // Evict oldest render results when over limit
            while (cache.size > MAX_CACHED_PAGES) {
                const oldestKey = cache.keys().next().value
                const oldEntry = cache.get(oldestKey)
                cache.delete(oldestKey)
                if (oldEntry?.src) URL.revokeObjectURL(oldEntry.src)
            }

            return url
        },
        createDocument: async () => {
            const page = await getPage(i)
            const doc = document.implementation.createHTMLDocument('')

            const canvas = doc.createElement('div')
            canvas.id = 'canvas'
            doc.body.appendChild(canvas)

            const textLayer = doc.createElement('div')
            textLayer.className = 'textLayer'
            doc.body.appendChild(textLayer)

            const annotationLayer = doc.createElement('div')
            annotationLayer.className = 'annotationLayer'
            doc.body.appendChild(annotationLayer)

            // TextLayer requires canvas 2d context for font metrics;
            // fall back to manual span construction when unavailable
            const probe = doc.createElement('canvas')
            if (probe.getContext?.('2d')) {
                const textLayerInstance = new pdfjsLib.TextLayer({
                    textContentSource: await page.streamTextContent(),
                    container: textLayer, viewport: page.getViewport({ scale: 1 }),
                })
                await textLayerInstance.render()
            } else {
                const content = await page.getTextContent()
                for (const item of content.items) {
                    if (item.str) {
                        const span = doc.createElement('span')
                        span.textContent = item.str
                        textLayer.appendChild(span)
                    }
                }
            }
            return doc
        },
        size: 1000,
    }))
    book.isExternal = uri => /^\w+:/i.test(uri)
    // TOC hrefs are JSON-encoded destinations (named or explicit); page-list
    // hrefs are JSON-encoded page indices.
    book.resolveHref = async href => {
        const parsed = JSON.parse(href)
        if (typeof parsed === 'number') return { index: parsed }
        const dest = typeof parsed === 'string'
            ? await pdf.getDestination(parsed) : parsed
        const index = await pdf.getPageIndex(dest[0])
        return { index }
    }
    book.splitTOCHref = async href => {
        if (!href) return [null, null]
        const parsed = JSON.parse(href)
        if (typeof parsed === 'number') return [parsed, null]
        const dest = typeof parsed === 'string'
            ? await pdf.getDestination(parsed) : parsed
        try {
            const index = await pdf.getPageIndex(dest[0])
            return [index, null]
        } catch (e) {
            console.warn('Error getting page index for href', href, e)
            return [null, null]
        }
    }
    book.getTOCFragment = doc => doc.documentElement
    book.getCover = async () => renderPage(await pdf.getPage(1), true)
    book.destroy = () => {
        // Clean up all cached canvases and revoke blob URLs
        for (const [, entry] of cache) {
            if (entry?.src) URL.revokeObjectURL(entry.src)
        }
        cache.clear()
        for (const [, page] of pageCache) {
            page?.cleanup()
        }
        pageCache.clear()
        loadingTask.destroy()
    }
    return book
}
