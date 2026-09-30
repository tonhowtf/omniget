// @ts-nocheck
// assign a unique ID for each TOC item
const assignIDs = toc => {
    let id = 0
    const assignID = item => {
        item.id = id++
        if (item.subitems) for (const subitem of item.subitems) assignID(subitem)
    }
    for (const item of toc) assignID(item)
    return toc
}

const flatten = items => items
    .map(item => item.subitems?.length
        ? [item, flatten(item.subitems)].flat()
        : item)
    .flat()

export class TOCProgress {
    async init({ toc, ids, splitHref, getFragment }) {
        assignIDs(toc)
        const items = flatten(toc)
        // splitHref is synchronous for EPUB (href.split) — awaiting it per
        // item used to schedule one microtask per TOC entry, thousands on
        // large collections. Batch instead; also parallelizes async providers.
        const splits = await Promise.all(items.map(item => splitHref(item?.href)))
        const grouped = new Map()
        for (const [i, item] of items.entries()) {
            const [id, fragment] = splits[i] ?? []
            const value = { fragment, item }
            if (grouped.has(id)) grouped.get(id).items.push(value)
            else grouped.set(id, { prev: items[i - 1], items: [value] })
        }
        const map = new Map()
        for (const [i, id] of ids.entries()) {
            if (grouped.has(id)) map.set(id, grouped.get(id))
            else map.set(id, map.get(ids[i - 1]))
        }
        this.ids = ids
        this.map = map
        this.getFragment = getFragment
    }
    getProgress(index, range) {
        if (!this.ids) return
        const id = this.ids[index]
        const obj = this.map.get(id)
        if (!obj) return null
        const { prev, items } = obj
        if (!items) return prev
        if (!range || items.length === 1 && !items[0].fragment) return items[0].item

        const doc = range.startContainer.getRootNode()
        for (const [i, { fragment }] of items.entries()) {
            const el = this.getFragment(doc, fragment)
            if (!el) continue
            if (range.comparePoint(el, 0) > 0)
                return (items[i - 1]?.item ?? prev)
        }
        return items[items.length - 1].item
    }
}

export class PageProgress {
    #book
    #cache = new Map()
    #resolveNavigation

    constructor(book, resolveNavigation) {
        this.#book = book
        this.#resolveNavigation = resolveNavigation
    }

    async #getCache(index) {
        let cached = this.#cache.get(index)
        if (cached) return cached

        const section = this.#book.sections[index]
        if (!section?.createDocument) return null

        const doc = await section.createDocument()
        const root = doc.body ?? doc.documentElement
        const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT)
        const nodes = []
        const offsets = []
        let total = 0
        for (let node = walker.nextNode(); node; node = walker.nextNode()) {
            const len = node.nodeValue?.length ?? 0
            nodes.push(node)
            offsets.push(total)
            total += len
        }

        cached = { doc, nodes, offsets, total }
        this.#cache.set(index, cached)
        return cached
    }

    async getProgress(cfi) {
        try {
            const nav = this.#resolveNavigation(cfi)
            if (!nav) return null

            const { index, anchor } = nav
            if (index == null || !anchor) return null

            const cached = await this.#getCache(index)
            if (!cached) return null

            const { doc, nodes, offsets, total } = cached
            const frag = anchor(doc)
            if (!frag) return null

            const isRange = frag instanceof Range
            const range = isRange ? frag : doc.createRange()
            if (!isRange) range.selectNodeContents(frag)

            const offset = this.#findOffset(range, nodes, offsets, total)
            return {
                fraction: total > 0 ? offset / total : 0,
                index,
            }
        } catch (e) {
            console.error(e)
            return null
        }
    }

    #findOffset(range, nodes, offsets, total) {
        if (!nodes.length) return 0
        const container = range.startContainer
        // fast path: startContainer is a text node in the index
        if (container.nodeType === Node.TEXT_NODE) {
            const i = this.#bsearchNode(container, nodes)
            if (i >= 0) return offsets[i] + range.startOffset
        }
        // element container: collapse to start and binary search
        const collapsed = range.cloneRange()
        collapsed.collapse(true)
        const i = this.#bsearchCollapsed(collapsed, nodes)
        return i >= 0 ? offsets[i] : total
    }

    // binary search for an exact text node by document position
    #bsearchNode(target, nodes) {
        let low = 0, high = nodes.length - 1
        while (low <= high) {
            const mid = (low + high) >> 1
            const node = nodes[mid]
            if (node === target) return mid
            const pos = node.compareDocumentPosition(target)
            if (pos & Node.DOCUMENT_POSITION_FOLLOWING) low = mid + 1
            else high = mid - 1
        }
        return -1
    }

    // binary search for the first text node at or after a collapsed range point
    // collapsed.comparePoint returns: -1 = node is before, 1 = node is at/after
    #bsearchCollapsed(collapsed, nodes) {
        let low = 0, high = nodes.length - 1, result = -1
        while (low <= high) {
            const mid = (low + high) >> 1
            if (collapsed.comparePoint(nodes[mid], 0) > 0) {
                result = mid
                high = mid - 1
            } else {
                low = mid + 1
            }
        }
        return result
    }
}

export class SectionProgress {
    constructor(sections, sizePerLoc, sizePerTimeUnit) {
        this.sizes = sections.map(s => s.linear != 'no' && s.size > 0 ? s.size : 0)
        this.sizePerLoc = sizePerLoc
        this.sizePerTimeUnit = sizePerTimeUnit
        this.sizeTotal = this.sizes.reduce((a, b) => a + b, 0)
        this.sectionFractions = this.#getSectionFractions()
    }
    #getSectionFractions() {
        const { sizeTotal } = this
        const results = [0]
        let sum = 0
        for (const size of this.sizes) results.push((sum += size) / sizeTotal)
        return results
    }
    // get progress given index of and fractions within a section
    getProgress(index, fractionInSection, pageFraction = 0) {
        const { sizes, sizePerLoc, sizePerTimeUnit, sizeTotal } = this
        const sizeInSection = sizes[index] ?? 0
        const sizeBefore = sizes.slice(0, index).reduce((a, b) => a + b, 0)
        const size = sizeBefore + fractionInSection * sizeInSection
        const nextSize = size + pageFraction * sizeInSection
        const remainingTotal = sizeTotal - size
        const remainingSection = (1 - fractionInSection) * sizeInSection
        return {
            fraction: nextSize / sizeTotal,
            section: {
                current: index,
                total: sizes.length,
            },
            location: {
                current: Math.floor(size / sizePerLoc),
                next: Math.floor(nextSize / sizePerLoc),
                total: Math.ceil(sizeTotal / sizePerLoc),
            },
            time: {
                section: remainingSection / sizePerTimeUnit,
                total: remainingTotal / sizePerTimeUnit,
            },
        }
    }
    // the inverse of `getProgress`
    // get index of and fraction in section based on total fraction
    getSection(fraction) {
        if (fraction <= 0) return [0, 0]
        if (fraction >= 1) return [this.sizes.length - 1, 1]
        fraction = fraction + Number.EPSILON
        const { sizeTotal } = this
        let index = this.sectionFractions.findIndex(x => x > fraction) - 1
        if (index < 0) return [0, 0]
        while (!this.sizes[index]) index++
        const fractionInSection = (fraction - this.sectionFractions[index])
            / (this.sizes[index] / sizeTotal)
        return [index, fractionInSection]
    }
}
