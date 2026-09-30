// @ts-nocheck
const createSVGElement = tag =>
    document.createElementNS('http://www.w3.org/2000/svg', tag)

let overlayerCounter = 0

// The page a rect sits in. The paginator sizes the root to a single page and
// lets the rest overflow it, so the root box tiles the pages along either
// axis, in either direction; in scrolled mode the root is the whole document.
const pageOf = (root, { left, top, right, bottom }) => {
    if (!(root.width > 0 && root.height > 0)) return null
    const x = root.left + Math.floor(((left + right) / 2 - root.left) / root.width) * root.width
    const y = root.top + Math.floor(((top + bottom) / 2 - root.top) / root.height) * root.height
    return { left: x, top: y, right: x + root.width, bottom: y + root.height }
}

export class Overlayer {
    #svg = createSVGElement('svg')
    #map = new Map()
    #doc = null
    #clipPath = null
    #clipPathPath = null
    #clipPathId

    constructor(doc) {
        this.#doc = doc
        this.#clipPathId = `foliate-loupe-clip-${overlayerCounter++}`
        Object.assign(this.#svg.style, {
            position: 'absolute', top: '0', left: '0',
            width: '100%', height: '100%',
            pointerEvents: 'none',
        })

        // Create a clipPath to cut a hole for the loupe.
        // We use clip-rule="evenodd" with a large outer rect and inner circle
        // to create the hole effect efficiently without mask compositing.
        const defs = createSVGElement('defs')
        this.#clipPath = createSVGElement('clipPath')
        this.#clipPath.setAttribute('id', this.#clipPathId)
        this.#clipPath.setAttribute('clipPathUnits', 'userSpaceOnUse')

        this.#clipPathPath = createSVGElement('path')
        this.#clipPathPath.setAttribute('clip-rule', 'evenodd')
        this.#clipPathPath.setAttribute('fill-rule', 'evenodd') // for older renderers

        this.#clipPath.append(this.#clipPathPath)
        defs.append(this.#clipPath)
        this.#svg.append(defs)
    }

    setHole(cx, cy, w, h, r) {
        // Define a path with a large outer rect and a capsule-shaped hole.
        // The capsule is a rounded rectangle (stadium shape) centred at (cx, cy).
        const outer = 'M -2000000 -2000000 H 4000000 V 4000000 H -2000000 Z'
        const hw = w / 2, hh = h / 2
        const cr = Math.min(r, hw, hh) // clamp corner radius
        const inner = `M ${cx - hw + cr} ${cy - hh}`
            + ` H ${cx + hw - cr}`
            + ` A ${cr} ${cr} 0 0 1 ${cx + hw} ${cy - hh + cr}`
            + ` V ${cy + hh - cr}`
            + ` A ${cr} ${cr} 0 0 1 ${cx + hw - cr} ${cy + hh}`
            + ` H ${cx - hw + cr}`
            + ` A ${cr} ${cr} 0 0 1 ${cx - hw} ${cy + hh - cr}`
            + ` V ${cy - hh + cr}`
            + ` A ${cr} ${cr} 0 0 1 ${cx - hw + cr} ${cy - hh} Z`
        this.#clipPathPath.setAttribute('d', `${outer} ${inner}`)

        this.#svg.setAttribute('clip-path', `url(#${this.#clipPathId})`)
        this.#svg.style.webkitClipPath = `url(#${this.#clipPathId})`
    }

    clearHole() {
        this.#svg.removeAttribute('clip-path')
        this.#svg.style.webkitClipPath = ''
        this.#clipPathPath.removeAttribute('d')
    }

    get element() {
        return this.#svg
    }
    get #zoom() {
        // Safari does not zoom the client rects, while Chrome, Edge and Firefox does
        if (/^((?!chrome|android).)*AppleWebKit/i.test(navigator.userAgent) && !window.chrome) {
            return window.getComputedStyle(this.#doc.body).zoom || 1.0
        }
        return 1.0
    }
    // Split a range into per-text-node sub-ranges (plus replaced elements
    // like images), so `getClientRects()` only ever returns line-level boxes.
    // Collecting rects on the whole range would also include the border boxes
    // of fully contained block elements, over-highlighting blank space.
    #splitRange(range) {
        const ancestor = range.commonAncestorContainer
        if (ancestor.nodeType !== Node.ELEMENT_NODE
            && ancestor.nodeType !== Node.DOCUMENT_NODE) return [range]
        const doc = ancestor.ownerDocument ?? ancestor
        const walker = doc.createTreeWalker(ancestor,
            NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT, {
                acceptNode: node => {
                    if (!range.intersectsNode(node)) return NodeFilter.FILTER_REJECT
                    // Ruby annotations sit on their own line above (or beside)
                    // the base, so their rects would draw a second detached box
                    // over the furigana. Never paint them — not the book's own
                    // ruby, not injected glosses.
                    const el = node.nodeType === Node.TEXT_NODE
                        ? node.parentElement : node
                    if (el?.closest?.('rt, rp, rtc, [cfi-inert]'))
                        return NodeFilter.FILTER_REJECT
                    if (node.nodeType === Node.TEXT_NODE) return NodeFilter.FILTER_ACCEPT
                    return node.matches?.('img, svg')
                        ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP
                },
            })
        const splitRanges = []
        for (let node = walker.nextNode(); node; node = walker.nextNode()) {
            const subRange = doc.createRange()
            if (node.nodeType === Node.TEXT_NODE) {
                subRange.selectNodeContents(node)
                if (subRange.compareBoundaryPoints(Range.START_TO_START, range) < 0) {
                    subRange.setStart(range.startContainer, range.startOffset)
                }
                if (subRange.compareBoundaryPoints(Range.END_TO_END, range) > 0) {
                    subRange.setEnd(range.endContainer, range.endOffset)
                }
            } else subRange.selectNode(node)
            splitRanges.push(subRange)
        }
        return splitRanges.length === 0 ? [range] : splitRanges
    }
    #getRects(range) {
        const zoom = this.#zoom
        const root = this.#doc.documentElement.getBoundingClientRect()
        const rects = []
        for (const subRange of this.#splitRange(range)) {
            for (const rect of subRange.getClientRects()) {
                const scaled = {
                    left: rect.left * zoom,
                    top: rect.top * zoom,
                    right: rect.right * zoom,
                    bottom: rect.bottom * zoom,
                    width: rect.width * zoom,
                    height: rect.height * zoom,
                }
                scaled.page = pageOf(root, scaled)
                rects.push(scaled)
            }
        }
        return rects
    }
    add(key, range, draw, options) {
        if (this.#map.has(key)) this.remove(key)
        if (typeof range === 'function') range = range(this.#svg.getRootNode())
        const rects = this.#getRects(range)
        const element = draw(rects, options)
        this.#svg.append(element)
        this.#map.set(key, { range, draw, options, element, rects })
    }
    remove(key) {
        if (!this.#map.has(key)) return
        this.#svg.removeChild(this.#map.get(key).element)
        this.#map.delete(key)
    }
    redraw() {
        for (const obj of this.#map.values()) {
            const { range, draw, options, element } = obj
            this.#svg.removeChild(element)
            const rects = this.#getRects(range)
            const el = draw(rects, options)
            this.#svg.append(el)
            obj.element = el
            obj.rects = rects
        }
    }
    hitTest({ x, y }) {
        const arr = Array.from(this.#map.entries())
        // loop in reverse to hit more recently added items first
        for (let i = arr.length - 1; i >= 0; i--) {
            const tolerance = 5
            const [key, obj] = arr[i]
            for (const { left, top, right, bottom } of obj.rects) {
                if (
                    top <= y + tolerance &&
                    left <= x + tolerance &&
                    bottom > y - tolerance &&
                    right > x - tolerance
                ) {
                    return [key, obj.range, { left, top, right, bottom }]
                }
            }
        }
        return []
    }
    static underline(rects, options = {}) {
        const { color = 'red', width: strokeWidth = 2, padding = 0, writingMode } = options
        const g = createSVGElement('g')
        g.setAttribute('fill', color)
        if (writingMode === 'vertical-rl' || writingMode === 'vertical-lr')
            for (const { right, top, height } of rects) {
                const el = createSVGElement('rect')
                el.setAttribute('x', right - strokeWidth / 2 + padding)
                el.setAttribute('y', top)
                el.setAttribute('height', height)
                el.setAttribute('width', strokeWidth)
                g.append(el)
            }
        else for (const { left, bottom, width } of rects) {
            const el = createSVGElement('rect')
            el.setAttribute('x', left)
            el.setAttribute('y', bottom - strokeWidth / 2 + padding)
            el.setAttribute('height', strokeWidth)
            el.setAttribute('width', width)
            g.append(el)
        }
        return g
    }
    static strikethrough(rects, options = {}) {
        const { color = 'red', width: strokeWidth = 2, writingMode } = options
        const g = createSVGElement('g')
        g.setAttribute('fill', color)
        if (writingMode === 'vertical-rl' || writingMode === 'vertical-lr')
            for (const { right, left, top, height } of rects) {
                const el = createSVGElement('rect')
                el.setAttribute('x', (right + left) / 2)
                el.setAttribute('y', top)
                el.setAttribute('height', height)
                el.setAttribute('width', strokeWidth)
                g.append(el)
            }
        else for (const { left, top, bottom, width } of rects) {
            const el = createSVGElement('rect')
            el.setAttribute('x', left)
            el.setAttribute('y', (top + bottom) / 2)
            el.setAttribute('height', strokeWidth)
            el.setAttribute('width', width)
            g.append(el)
        }
        return g
    }
    static squiggly(rects, options = {}) {
        const { color = 'red', width: strokeWidth = 2, padding = 0, writingMode } = options
        const g = createSVGElement('g')
        g.setAttribute('fill', 'none')
        g.setAttribute('stroke', color)
        g.setAttribute('stroke-width', strokeWidth)
        const block = strokeWidth * 1.5
        if (writingMode === 'vertical-rl' || writingMode === 'vertical-lr')
            for (const { right, top, height } of rects) {
                const el = createSVGElement('path')
                const n = Math.round(height / block / 1.5)
                const inline = height / n
                const ls = Array.from({ length: n },
                    (_, i) => `l${i % 2 ? -block : block} ${inline}`).join('')
                el.setAttribute('d', `M${right - strokeWidth / 2 + padding} ${top}${ls}`)
                g.append(el)
            }
        else for (const { left, bottom, width } of rects) {
            const el = createSVGElement('path')
            const n = Math.round(width / block / 1.5)
            const inline = width / n
            const ls = Array.from({ length: n },
                (_, i) => `l${inline} ${i % 2 ? block : -block}`).join('')
            el.setAttribute('d', `M${left} ${bottom + strokeWidth / 2 + padding}${ls}`)
            g.append(el)
        }
        return g
    }
    static highlight(rects, options = {}) {
        const {
            color = 'red',
            padding = 0,
            radius = 4,
            radiusPadding = 2,
            vertical = false,
        } = options

        const g = createSVGElement('g')
        g.setAttribute('fill', color)
        g.style.opacity = 'var(--overlayer-highlight-opacity, .3)'
        g.style.mixBlendMode = 'var(--overlayer-highlight-blend-mode, normal)'

        for (const [index, { left, top, height, width, page }] of rects.entries()) {
            const isFirst = index === 0
            const isLast = index === rects.length - 1

            let x, y, w, h

            let radiusTopLeft, radiusTopRight, radiusBottomRight, radiusBottomLeft

            if (vertical) {
                x = left - padding
                y = top - padding - (isFirst ? radiusPadding : 0)
                w = width + padding * 2
                h = height + padding * 2 + (isFirst ? radiusPadding : 0) + (isLast ? radiusPadding : 0)
                radiusTopLeft = isFirst ? radius : 0
                radiusTopRight = isFirst ? radius : 0
                radiusBottomRight = isLast ? radius : 0
                radiusBottomLeft = isLast ? radius : 0
            } else {
                x = left - padding - (isFirst ? radiusPadding : 0)
                y = top - padding
                w = width + padding * 2 + (isFirst ? radiusPadding : 0) + (isLast ? radiusPadding : 0)
                h = height + padding * 2
                radiusTopLeft = isFirst ? radius : 0
                radiusTopRight = isLast ? radius : 0
                radiusBottomRight = isLast ? radius : 0
                radiusBottomLeft = isFirst ? radius : 0
            }

            // The caps pad past the rects, and with the page margins and gap
            // at zero the pages touch: the 2px after a column-wide image
            // painted a stripe the height of the image down the edge of the
            // next page (readest/readest#6128). Never paint past the rect's
            // own page.
            if (page) {
                const right = Math.min(x + w, page.right)
                const bottom = Math.min(y + h, page.bottom)
                x = Math.max(x, page.left)
                y = Math.max(y, page.top)
                w = right - x
                h = bottom - y
                if (w <= 0 || h <= 0) continue
            }

            const rtl = Math.min(radiusTopLeft, w / 2, h / 2)
            const rtr = Math.min(radiusTopRight, w / 2, h / 2)
            const rbr = Math.min(radiusBottomRight, w / 2, h / 2)
            const rbl = Math.min(radiusBottomLeft, w / 2, h / 2)

            if (rtl === 0 && rtr === 0 && rbr === 0 && rbl === 0) {
                const el = createSVGElement('rect')
                el.setAttribute('x', x)
                el.setAttribute('y', y)
                el.setAttribute('height', h)
                el.setAttribute('width', w)
                g.append(el)
            } else {
                const el = createSVGElement('path')
                const d = `
                M ${x + rtl} ${y}
                L ${x + w - rtr} ${y}
                ${rtr > 0 ? `Q ${x + w} ${y} ${x + w} ${y + rtr}` : `L ${x + w} ${y}`}
                L ${x + w} ${y + h - rbr}
                ${rbr > 0 ? `Q ${x + w} ${y + h} ${x + w - rbr} ${y + h}` : `L ${x + w} ${y + h}`}
                L ${x + rbl} ${y + h}
                ${rbl > 0 ? `Q ${x} ${y + h} ${x} ${y + h - rbl}` : `L ${x} ${y + h}`}
                L ${x} ${y + rtl}
                ${rtl > 0 ? `Q ${x} ${y} ${x + rtl} ${y}` : `L ${x} ${y}`}
                Z
            `.trim().replace(/\s+/g, ' ')
                el.setAttribute('d', d)
                g.append(el)
            }
        }
        return g
    }
    static outline(rects, options = {}) {
        const { color = 'red', width: strokeWidth = 3, padding = 0, radius = 3 } = options
        const g = createSVGElement('g')
        g.setAttribute('fill', 'none')
        g.setAttribute('stroke', color)
        g.setAttribute('stroke-width', strokeWidth)
        for (const { left, top, height, width } of rects) {
            const el = createSVGElement('rect')
            el.setAttribute('x', left - padding)
            el.setAttribute('y', top - padding)
            el.setAttribute('height', height + padding * 2)
            el.setAttribute('width', width + padding * 2)
            el.setAttribute('rx', radius)
            g.append(el)
        }
        return g
    }
    static bubble(rects, options = {}) {
        const { color = '#fbbf24', writingMode, opacity = 0.85, size = 20, padding = 10 } = options
        const isVertical = writingMode === 'vertical-rl' || writingMode === 'vertical-lr'
        const g = createSVGElement('g')
        g.style.opacity = opacity
        if (rects.length === 0) return g
        rects.splice(1)
        const firstRect = rects[0]
        const x = isVertical ? firstRect.right - size + padding : firstRect.right - size + padding
        const y = isVertical ? firstRect.bottom - size + padding : firstRect.top - size + padding
        firstRect.top = y - padding
        firstRect.right = x + size + padding
        firstRect.bottom = y + size + padding
        firstRect.left = x - padding
        const bubble = createSVGElement('path')
        const s = size
        const r = s * 0.15
        // Speech bubble shape with a small tail
        // Main rounded rectangle body
        const d = `
            M ${x + r} ${y}
            h ${s - 2 * r}
            a ${r} ${r} 0 0 1 ${r} ${r}
            v ${s * 0.65 - 2 * r}
            a ${r} ${r} 0 0 1 ${-r} ${r}
            h ${-s * 0.3}
            l ${-s * 0.15} ${s * 0.2}
            l ${s * 0.05} ${-s * 0.2}
            h ${-s * 0.6 + 2 * r}
            a ${r} ${r} 0 0 1 ${-r} ${-r}
            v ${-s * 0.65 + 2 * r}
            a ${r} ${r} 0 0 1 ${r} ${-r}
            z
        `.replace(/\s+/g, ' ').trim()

        bubble.setAttribute('d', d)
        bubble.setAttribute('fill', color)
        bubble.setAttribute('stroke', 'rgba(0, 0, 0, 0.2)')
        bubble.setAttribute('stroke-width', '1')
        // Add horizontal lines inside to represent text
        const lineGroup = createSVGElement('g')
        lineGroup.setAttribute('stroke', 'rgba(0, 0, 0, 0.3)')
        lineGroup.setAttribute('stroke-width', '1.5')
        lineGroup.setAttribute('stroke-linecap', 'round')
        const lineY1 = y + s * 0.18
        const lineY2 = y + s * 0.33
        const lineY3 = y + s * 0.48
        const lineX1 = x + s * 0.2
        const lineX2 = x + s * 0.8
        const line1 = createSVGElement('line')
        line1.setAttribute('x1', lineX1)
        line1.setAttribute('y1', lineY1)
        line1.setAttribute('x2', lineX2)
        line1.setAttribute('y2', lineY1)
        const line2 = createSVGElement('line')
        line2.setAttribute('x1', lineX1)
        line2.setAttribute('y1', lineY2)
        line2.setAttribute('x2', lineX2)
        line2.setAttribute('y2', lineY2)
        const line3 = createSVGElement('line')
        line3.setAttribute('x1', lineX1)
        line3.setAttribute('y1', lineY3)
        line3.setAttribute('x2', x + s * 0.6)
        line3.setAttribute('y2', lineY3)
        lineGroup.append(line1, line2, line3)

        if (isVertical) {
            const centerX = x + s / 2
            const centerY = y + s / 2
            bubble.setAttribute('transform', `rotate(90 ${centerX} ${centerY})`)
            lineGroup.setAttribute('transform', `rotate(90 ${centerX} ${centerY})`)
        }

        g.append(bubble)
        g.append(lineGroup)
        return g
    }
    // make an exact copy of an image in the overlay
    // one can then apply filters to the entire element, without affecting them;
    // it's a bit silly and probably better to just invert images twice
    // (though the color will be off in that case if you do heu-rotate)
    static copyImage([rect], options = {}) {
        const { src } = options
        const image = createSVGElement('image')
        const { left, top, height, width } = rect
        image.setAttribute('href', src)
        image.setAttribute('x', left)
        image.setAttribute('y', top)
        image.setAttribute('height', height)
        image.setAttribute('width', width)
        return image
    }
}

