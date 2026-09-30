// @ts-nocheck
const NS = {
    XML: 'http://www.w3.org/XML/1998/namespace',
    SSML: 'http://www.w3.org/2001/10/synthesis',
}

const blockTags = new Set([
    'article', 'aside', 'audio', 'blockquote', 'caption',
    'details', 'dialog', 'div', 'dl', 'dt', 'dd',
    'figure', 'footer', 'form', 'figcaption',
    'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'header', 'hgroup', 'hr', 'li',
    'main', 'math', 'nav', 'ol', 'p', 'pre', 'section', 'tr',
])

const getLang = el => {
    const x = el.lang || el?.getAttributeNS?.(NS.XML, 'lang')
    return x ? x : el.parentElement ? getLang(el.parentElement) : null
}

const getAlphabet = el => {
    const x = el?.getAttributeNS?.(NS.XML, 'lang')
    return x ? x : el.parentElement ? getAlphabet(el.parentElement) : null
}

const getSegmenter = (lang, granularity = 'word') => {
    const segmenter = new Intl.Segmenter(lang || undefined, { granularity })
    const granularityIsWord = granularity === 'word'
    return function* (strs, makeRange) {
        const str = strs.join('').replace(/\r\n/g, '  ').replace(/\r/g, ' ').replace(/\n/g, ' ')
        let name = 0
        let strIndex = -1
        let sum = 0
        const rawSegments = Array.from(segmenter.segment(str))
        const mergedSegments = []
        for (let i = 0, j = 0; i < rawSegments.length; i++) {
            const current = rawSegments[i]
            const segment = ' ' + current.segment
            const endsWithAbbr = /\s([A-Z]{1,2}[a-z]{0,5}|[a-z]{1,3})\.\s*$/.test(segment)
            if (!endsWithAbbr || i >= (rawSegments.length-1)) {
                const mergedSegment = {
                    index: rawSegments[j].index,
                    segment: '',
                    isWordLike: (i == j) ? current.isWordLike : true,
                }
                while (j <= i) {
                    mergedSegment.segment += rawSegments[j++].segment
                }
                mergedSegments.push(mergedSegment)
            }
        }

        for (const { index, segment, isWordLike } of mergedSegments) {
            if (granularityIsWord && !isWordLike) continue
            while (sum <= index) sum += strs[++strIndex].length
            const startIndex = strIndex
            const startOffset = index - (sum - strs[strIndex].length)
            const end = index + segment.length - 1
            if (end < str.length) while (sum <= end) sum += strs[++strIndex].length
            const endIndex = strIndex
            const endOffset = end - (sum - strs[strIndex].length) + 1
            yield [(name++).toString(),
                makeRange(startIndex, startOffset, endIndex, endOffset)]
        }
    }
}

const fragmentToSSML = (fragment, nodeFilter, inherited) => {
    const ssml = document.implementation.createDocument(NS.SSML, 'speak')
    const { lang } = inherited
    if (lang) ssml.documentElement.setAttributeNS(NS.XML, 'lang', lang)

    const convert = (node, parent, inheritedAlphabet) => {
        if (!node) return
        // Text nodes go through the filter too: the text walker that produces
        // the marks already honours it, and content that is skipped there (a
        // bare ruby base inside <ruby>, say) must not reach the speech either,
        // or the two disagree about what is being read.
        if (node.nodeType === 3 || node.nodeType === 4) {
            if (nodeFilter && nodeFilter(node) === NodeFilter.FILTER_REJECT) return
            return node.nodeType === 3
                ? ssml.createTextNode(node.textContent)
                : ssml.createCDATASection(node.textContent)
        }
        if (node.nodeType !== 1 && node.nodeType !== 11) return
        if (nodeFilter && nodeFilter(node) === NodeFilter.FILTER_REJECT) return

        let el
        const nodeName = node.nodeName.toLowerCase()
        if (nodeName === 'foliate-mark') {
            el = ssml.createElementNS(NS.SSML, 'mark')
            el.setAttribute('name', node.dataset.name)
        }
        else if (nodeName === 'br')
            el = ssml.createElementNS(NS.SSML, 'break')
        else if (nodeName === 'em' || nodeName === 'strong')
            el = ssml.createElementNS(NS.SSML, 'emphasis')

        const lang = node.lang || node.getAttributeNS?.(NS.XML, 'lang')
        if (lang) {
            if (!el) el = ssml.createElementNS(NS.SSML, 'lang')
            el.setAttributeNS(NS.XML, 'lang', lang)
        }

        const alphabet = node.getAttributeNS?.(NS.SSML, 'alphabet') || inheritedAlphabet
        if (!el) {
            const ph = node.getAttributeNS?.(NS.SSML, 'ph')
            if (ph) {
                el = ssml.createElementNS(NS.SSML, 'phoneme')
                if (alphabet) el.setAttribute('alphabet', alphabet)
                el.setAttribute('ph', ph)
            }
        }

        if (!el) el = parent

        let child = node.firstChild
        while (child) {
            const childEl = convert(child, el, alphabet)
            if (childEl && el !== childEl) el.append(childEl)
            child = child.nextSibling
        }
        return el
    }
    convert(fragment, ssml.documentElement, inherited.alphabet)
    return ssml
}

const getFragmentWithMarks = (range, textWalker, nodeFilter, granularity) => {
    const lang = getLang(range.commonAncestorContainer)
    const alphabet = getAlphabet(range.commonAncestorContainer)

    const segmenter = getSegmenter(lang, granularity)
    const fragment = range.cloneContents()

    // we need ranges on both the original document (for highlighting)
    // and the document fragment (for inserting marks)
    // so unfortunately need to do it twice, as you can't copy the ranges
    const entries = [...textWalker(range, segmenter, nodeFilter)]
    const fragmentEntries = [...textWalker(fragment, segmenter, nodeFilter)]

    for (const [name, range] of fragmentEntries) {
        const mark = document.createElement('foliate-mark')
        mark.dataset.name = name
        range.insertNode(mark)
    }
    const ssml = fragmentToSSML(fragment, nodeFilter, { lang, alphabet })
    return { entries, ssml }
}

const rangeIsEmpty = range => !range.toString().trim()

// For PDF text layers, split content into sentence-level blocks so TTS
// reads one sentence at a time instead of the whole page in one block.
// Text nodes are split at sentence boundaries so that every block range
// aligns with node edges — this prevents the text walker from including
// text outside the sentence in word marks.
function* getPDFSentenceBlocks(doc, textLayer) {
    const collectNodes = () => {
        const w = doc.createTreeWalker(textLayer, NodeFilter.SHOW_TEXT)
        const res = []
        for (let n = w.nextNode(); n; n = w.nextNode()) res.push(n)
        return res
    }

    let nodes = collectNodes()
    if (!nodes.length) return

    const fullText = nodes.map(n => n.nodeValue).join('')
    if (!fullText.trim()) return

    // Find sentence boundary positions
    const lang = getLang(textLayer) || undefined
    const segmenter = new Intl.Segmenter(lang, { granularity: 'sentence' })
    const boundaries = new Set()
    for (const { index } of segmenter.segment(fullText))
        if (index > 0) boundaries.add(index)

    // Split text nodes at sentence boundaries so ranges align with node edges.
    // Process in reverse order to preserve earlier character positions.
    let cum = 0
    const nodeStarts = nodes.map(n => { const s = cum; cum += n.nodeValue.length; return s })

    for (const pos of [...boundaries].sort((a, b) => b - a)) {
        for (let i = 0; i < nodes.length; i++) {
            const start = nodeStarts[i]
            const end = start + nodes[i].nodeValue.length
            if (pos > start && pos < end) {
                nodes[i].splitText(pos - start)
                break
            }
        }
    }

    // Re-collect nodes after splits and group into sentence blocks
    nodes = collectNodes()
    cum = 0
    let groupStart = 0
    let blockCount = 0

    for (let i = 0; i < nodes.length; i++) {
        cum += nodes[i].nodeValue.length
        const isEnd = i === nodes.length - 1 || boundaries.has(cum)
        if (isEnd) {
            const range = doc.createRange()
            range.setStart(nodes[groupStart], 0)
            range.setEnd(nodes[i], nodes[i].nodeValue.length)
            if (!rangeIsEmpty(range)) {
                blockCount++
                yield range
            }
            groupStart = i + 1
        }
    }
}

function* getBlocks(doc, nodeFilter) {
    const root = doc.body
        ?? doc.querySelector('body')
        ?? doc.documentElement

    // For PDF text layers, yield sentence-level blocks
    const textLayer = root.querySelector?.('.textLayer')
    if (textLayer) {
        yield* getPDFSentenceBlocks(doc, textLayer)
        return
    }

    let last
    let sawBlock = false
    let sawSkipped = false
    const walker = doc.createTreeWalker(root, NodeFilter.SHOW_ELEMENT)
    let node = walker.nextNode()
    while (node) {
        const name = node.tagName.toLowerCase()
        // A rejected block element (e.g. a footnote/endnote aside) must not be
        // read: skip its whole subtree and end the preceding block before it
        // so its text doesn't leak into the adjacent block. Inline rejects are
        // left to the text walker in getFragmentWithMarks().
        if (blockTags.has(name)
                && nodeFilter?.(node) === NodeFilter.FILTER_REJECT) {
            sawSkipped = true
            if (last) {
                last.setEndBefore(node)
                if (!rangeIsEmpty(last)) yield last
                last = null
            }
            const skipped = node
            do node = walker.nextNode()
            while (node && (skipped.compareDocumentPosition(node)
                & Node.DOCUMENT_POSITION_CONTAINED_BY))
            continue
        }
        if (blockTags.has(name)) {
            if (last) {
                last.setEndBefore(node)
                if (!rangeIsEmpty(last)) yield last
            }
            last = doc.createRange()
            last.setStart(node, 0)
            sawBlock = true
        }
        node = walker.nextNode()
    }
    if (last) {
        last.setEndAfter(root.lastChild ?? root)
        if (!rangeIsEmpty(last)) yield last
    } else if (!sawBlock && !sawSkipped) {
        last = doc.createRange()
        last.setStart(root.firstChild ?? root, 0)
        last.setEndAfter(root.lastChild ?? root)
        if (!rangeIsEmpty(last)) yield last
    }
}

// Enumerate every TTS segment of the document in order without touching any
// TTS instance state. blockIndex/markName match what a TTS instance produces
// for the same granularity, so callers (e.g. a playback timeline) can
// correlate the enumeration with live marks and use each range with from().
export function* getSentences(doc, textWalker, nodeFilter, granularity = 'sentence') {
    let blockIndex = 0
    for (const range of getBlocks(doc, nodeFilter)) {
        const lang = getLang(range.commonAncestorContainer)
        const segmenter = getSegmenter(lang, granularity)
        for (const [name, segRange] of textWalker(range, segmenter, nodeFilter))
            yield { blockIndex, markName: name, range: segRange }
        blockIndex++
    }
}

class ListIterator {
    #arr = []
    #iter
    #index = -1
    #f
    constructor(iter, f = x => x) {
        this.#iter = iter
        this.#f = f
    }
    current() {
        if (this.#arr[this.#index]) return this.#f(this.#arr[this.#index])
    }
    first() {
        const newIndex = 0
        if (this.#arr[newIndex]) {
            this.#index = newIndex
            return this.#f(this.#arr[newIndex])
        }
    }
    prev() {
        const newIndex = this.#index - 1
        if (this.#arr[newIndex]) {
            this.#index = newIndex
            return this.#f(this.#arr[newIndex])
        }
    }
    next() {
        const newIndex = this.#index + 1
        if (this.#arr[newIndex]) {
            this.#index = newIndex
            return this.#f(this.#arr[newIndex])
        }
        while (true) {
            const { done, value } = this.#iter.next()
            if (done) break
            this.#arr.push(value)
            if (this.#arr[newIndex]) {
                this.#index = newIndex
                return this.#f(this.#arr[newIndex])
            }
        }
    }
    find(f) {
        const index = this.#arr.findIndex(x => f(x))
        if (index > -1) {
            this.#index = index
            return this.#f(this.#arr[index])
        }
        while (true) {
            const { done, value } = this.#iter.next()
            if (done) break
            this.#arr.push(value)
            if (f(value)) {
                this.#index = this.#arr.length - 1
                return this.#f(value)
            }
        }
    }
}

export class TTS {
    #list
    #ranges
    #lastMark
    #serializer = new XMLSerializer()
    constructor(doc, textWalker, nodeFilter, highlight, granularity) {
        this.doc = doc
        this.highlight = highlight
        this.#list = new ListIterator(getBlocks(doc, nodeFilter), range => {
            const { entries, ssml } = getFragmentWithMarks(range, textWalker, nodeFilter, granularity)
            this.#ranges = new Map(entries)
            return [ssml, range]
        })
    }
    #getMarkElement(doc, mark) {
        if (!mark) return null
        return doc.querySelector(`mark[name="${CSS.escape(mark)}"`)
    }
    #speak(doc, getNode) {
        if (!doc) return
        if (!getNode) return this.#serializer.serializeToString(doc)
        const ssml = document.implementation.createDocument(NS.SSML, 'speak')
        ssml.documentElement.replaceWith(ssml.importNode(doc.documentElement, true))
        let node = getNode(ssml)?.previousSibling
        while (node) {
            const next = node.previousSibling ?? node.parentNode?.previousSibling
            node.parentNode.removeChild(node)
            node = next
        }
        const ssmlStr = this.#serializer.serializeToString(ssml)
        return ssmlStr
    }
    start() {
        this.#lastMark = null
        const [doc] = this.#list.first() ?? []
        if (!doc) return this.next()
        return this.#speak(doc, ssml => this.#getMarkElement(ssml, this.#lastMark))
    }
    resume() {
        const [doc] = this.#list.current() ?? []
        if (!doc) return this.next()
        return this.#speak(doc, ssml => this.#getMarkElement(ssml, this.#lastMark))
    }
    prev(paused) {
        this.#lastMark = null
        const [doc, range] = this.#list.prev() ?? []
        if (paused && range) this.highlight(range.cloneRange())
        return this.#speak(doc)
    }
    next(paused) {
        this.#lastMark = null
        const [doc, range] = this.#list.next() ?? []
        if (paused && range) this.highlight(range.cloneRange())
        return this.#speak(doc)
    }
    prevMark(paused) {
        const marks = Array.from(this.#ranges.keys())
        if (marks.length === 0) return

        const currentIndex = this.#lastMark ? marks.indexOf(this.#lastMark) : -1
        if (currentIndex > 0) {
            const prevMarkName = marks[currentIndex - 1]
            const range = this.#ranges.get(prevMarkName)
            if (range) {
                this.#lastMark = prevMarkName
                if (paused) this.highlight(range.cloneRange())

                const [doc] = this.#list.current() ?? []
                return this.#speak(doc, ssml => this.#getMarkElement(ssml, prevMarkName))
            }
        } else {
            const [doc, range] = this.#list.prev() ?? []
            if (doc && range) {
                const prevMarks = Array.from(this.#ranges.keys())
                if (prevMarks.length > 0) {
                    const lastMarkName = prevMarks[prevMarks.length - 1]
                    const lastMarkRange = this.#ranges.get(lastMarkName)
                    if (lastMarkRange) {
                        this.#lastMark = lastMarkName
                        if (paused) this.highlight(lastMarkRange.cloneRange())
                        return this.#speak(doc, ssml => this.#getMarkElement(ssml, lastMarkName))
                    }
                } else {
                    this.#lastMark = null
                    if (paused) this.highlight(range.cloneRange())
                    return this.#speak(doc)
                }
            }
        }
    }
    nextMark(paused) {
        const marks = Array.from(this.#ranges.keys())
        if (marks.length === 0) return

        const currentIndex = this.#lastMark ? marks.indexOf(this.#lastMark) : -1
        if (currentIndex >= 0 && currentIndex < marks.length - 1) {
            const nextMarkName = marks[currentIndex + 1]
            const range = this.#ranges.get(nextMarkName)
            if (range) {
                this.#lastMark = nextMarkName
                if (paused) this.highlight(range.cloneRange())
                const [doc] = this.#list.current() ?? []
                return this.#speak(doc, ssml => this.#getMarkElement(ssml, nextMarkName))
            }
        } else {
            const [doc, range] = this.#list.next() ?? []
            if (doc && range) {
                const nextMarks = Array.from(this.#ranges.keys())
                if (nextMarks.length > 0) {
                    const firstMarkName = nextMarks[0]
                    const firstMarkRange = this.#ranges.get(firstMarkName)
                    if (firstMarkRange) {
                        this.#lastMark = firstMarkName
                        if (paused) this.highlight(firstMarkRange.cloneRange())
                        return this.#speak(doc, ssml => this.#getMarkElement(ssml, firstMarkName))
                    }
                } else {
                    this.#lastMark = null
                    if (paused) this.highlight(range.cloneRange())
                    return this.#speak(doc)
                }
            }
        }
    }
    from(range) {
        this.#lastMark = null
        const [doc] = this.#list.find(range_ =>
            range.compareBoundaryPoints(Range.END_TO_START, range_) <= 0)
        // Pick the mark whose sentence contains the selection: the last mark
        // that begins at or before it. Taking the first mark beginning at or
        // after the selection skipped to the next sentence whenever the
        // selected word was not its sentence's first word.
        let mark
        for (const [name, range_] of this.#ranges.entries()) {
            if (range.compareBoundaryPoints(Range.START_TO_START, range_) < 0) break
            mark = name
        }
        return this.#speak(doc, ssml => this.#getMarkElement(ssml, mark))
    }
    getLastRange() {
        if (this.#lastMark) {
            const range = this.#ranges.get(this.#lastMark)
            if (range) return range.cloneRange()
        }
    }
    setMark(mark) {
        const range = this.#ranges.get(mark)
        if (range) {
            this.#lastMark = mark
            this.highlight(range.cloneRange())
            return range
        }
    }
}
