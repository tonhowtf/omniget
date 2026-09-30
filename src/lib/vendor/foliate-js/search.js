// @ts-nocheck
// length for context in excerpts
const CONTEXT_LENGTH = 50

const normalizeWhitespace = str => str.replace(/\s+/g, ' ')

// Gather context preceding the match by walking back across text nodes until we
// have enough for the excerpt. A match can sit in its own text node (e.g. a word
// wrapped in <i>/<em>/<b>), leaving no context within the start node itself.
const collectBefore = (strs, index, offset) => {
    let str = strs[index].slice(0, offset)
    for (let i = index - 1; i >= 0 && normalizeWhitespace(str).trim().length < CONTEXT_LENGTH; i--)
        str = strs[i] + str
    return str
}

const collectAfter = (strs, index, offset) => {
    let str = strs[index].slice(offset)
    for (let i = index + 1; i < strs.length && normalizeWhitespace(str).trim().length < CONTEXT_LENGTH; i++)
        str += strs[i]
    return str
}

const makeExcerpt = (strs, { startIndex, startOffset, endIndex, endOffset }) => {
    const start = strs[startIndex]
    const end = strs[endIndex]
    const match = startIndex === endIndex
        ? start.slice(startOffset, endOffset)
        : start.slice(startOffset)
            + strs.slice(startIndex + 1, endIndex).join('')
            + end.slice(0, endOffset)
    const trimmedStart = normalizeWhitespace(collectBefore(strs, startIndex, startOffset)).trimStart()
    const trimmedEnd = normalizeWhitespace(collectAfter(strs, endIndex, endOffset)).trimEnd()
    const ellipsisPre = trimmedStart.length < CONTEXT_LENGTH ? '' : '…'
    const ellipsisPost = trimmedEnd.length < CONTEXT_LENGTH ? '' : '…'
    const pre = `${ellipsisPre}${trimmedStart.slice(-CONTEXT_LENGTH)}`
    const post = `${trimmedEnd.slice(0, CONTEXT_LENGTH)}${ellipsisPost}`
    return { pre, match, post }
}

// Cumulative character offsets of the joined `strs`, so a flat offset into
// `strs.join('')` can be mapped back to a (node index, in-node offset) pair.
const buildCum = strs => {
    const cum = [0]
    for (let i = 0; i < strs.length; i++) cum.push(cum[i] + strs[i].length)
    return cum
}

// Largest node i with cum[i] <= offset; clamps the end-of-text offset to the
// last node's end. Works for both range start and (exclusive) end positions.
const nodeAt = (cum, offset) => {
    let lo = 0, hi = cum.length - 2
    if (hi < 0) return { index: 0, offset: 0 }
    while (lo < hi) {
        const mid = (lo + hi + 1) >> 1
        if (cum[mid] <= offset) lo = mid
        else hi = mid - 1
    }
    return { index: lo, offset: offset - cum[lo] }
}

const rangeFromFlat = (cum, start, end) => {
    const s = nodeAt(cum, start)
    const e = nodeAt(cum, end)
    return { startIndex: s.index, startOffset: s.offset, endIndex: e.index, endOffset: e.offset }
}

const simpleSearch = function* (strs, query, options = {}) {
    const { locales = 'en', sensitivity } = options
    const matchCase = sensitivity === 'variant'
    const haystack = strs.join('')
    const lowerHaystack = matchCase ? haystack : haystack.toLocaleLowerCase(locales)
    const needle = matchCase ? query : query.toLocaleLowerCase(locales)
    const needleLength = needle.length
    let index = -1
    let strIndex = -1
    let sum = 0
    do {
        index = lowerHaystack.indexOf(needle, index + 1)
        if (index > -1) {
            while (sum <= index) sum += strs[++strIndex].length
            const startIndex = strIndex
            const startOffset = index - (sum - strs[strIndex].length)
            const end = index + needleLength
            while (sum <= end) sum += strs[++strIndex].length
            const endIndex = strIndex
            const endOffset = end - (sum - strs[strIndex].length)
            const range = { startIndex, startOffset, endIndex, endOffset }
            yield { range, excerpt: makeExcerpt(strs, range) }
        }
    } while (index > -1)
}

const segmenterSearch = function* (strs, query, options = {}) {
    const { locales = 'en', granularity = 'word', sensitivity = 'base' } = options
    let segmenter, collator
    try {
        segmenter = new Intl.Segmenter(locales, { usage: 'search', granularity })
        collator = new Intl.Collator(locales, { sensitivity })
    } catch (e) {
        console.warn(e)
        segmenter = new Intl.Segmenter('en', { usage: 'search', granularity })
        collator = new Intl.Collator('en', { sensitivity })
    }
    const queryLength = Array.from(segmenter.segment(query)).length

    const substrArr = []
    let strIndex = 0
    let segments = segmenter.segment(strs[strIndex])[Symbol.iterator]()
    main: while (strIndex < strs.length) {
        while (substrArr.length < queryLength) {
            const { done, value } = segments.next()
            if (done) {
                // the current string is exhausted
                // move on to the next string
                strIndex++
                if (strIndex < strs.length) {
                    segments = segmenter.segment(strs[strIndex])[Symbol.iterator]()
                    continue
                } else break main
            }
            const { index, segment } = value
            // ignore formatting characters
            if (!/[^\p{Format}]/u.test(segment)) continue
            // normalize whitespace
            if (/\s/u.test(segment)) {
                if (!/\s/u.test(substrArr[substrArr.length - 1]?.segment))
                    substrArr.push({ strIndex, index, segment: ' ' })
                continue
            }
            value.strIndex = strIndex
            substrArr.push(value)
        }
        const substr = substrArr.map(x => x.segment).join('')
        if (collator.compare(query, substr) === 0) {
            const endIndex = strIndex
            const lastSeg = substrArr[substrArr.length - 1]
            const endOffset = lastSeg.index + lastSeg.segment.length
            const startIndex = substrArr[0].strIndex
            const startOffset = substrArr[0].index
            const range = { startIndex, startOffset, endIndex, endOffset }
            yield { range, excerpt: makeExcerpt(strs, range) }
        }
        substrArr.shift()
    }
}

// Calibre-parity regex mode (#4560). Runs a JS RegExp over the joined text and
// maps each match back to a node range. Note: RegExp.exec is synchronous, so a
// catastrophic pattern can still stall this pass — true interruption (a Web
// Worker) is left to a follow-up; here we only cap match count and reject
// invalid patterns. The caller surfaces INVALID_REGEX as a calm inline error.
const MAX_REGEX_MATCHES = 10000
const regexSearch = function* (strs, query, options = {}) {
    const { matchCase } = options
    const flags = matchCase ? 'g' : 'gi'
    let re
    try {
        re = new RegExp(query, flags + 'u')
    } catch {
        // Some patterns are valid only without the unicode flag; fall back.
        try {
            re = new RegExp(query, flags)
        } catch (e) {
            const err = new Error(`Invalid regular expression: ${e.message}`)
            err.code = 'INVALID_REGEX'
            throw err
        }
    }
    const haystack = strs.join('')
    const cum = buildCum(strs)
    let count = 0
    let m
    while ((m = re.exec(haystack)) !== null) {
        if (m[0].length === 0) {
            // zero-width match: advance to avoid an infinite loop
            re.lastIndex = m.index + 1
            continue
        }
        const range = rangeFromFlat(cum, m.index, m.index + m[0].length)
        yield { range, excerpt: makeExcerpt(strs, range) }
        if (++count >= MAX_REGEX_MATCHES) break
    }
}

// Segmented excerpt for nearby-words: emphasizes only the matched words inside
// the cluster window, leaving the gap text un-emphasized. `pre`/`match`/`post`
// stay populated for consumers that don't render segments.
const makeNearbyExcerpt = (haystack, matched) => {
    const clusterStart = matched[0].start
    const clusterEnd = matched[matched.length - 1].end
    const trimmedStart = normalizeWhitespace(haystack.slice(0, clusterStart)).trimStart()
    const trimmedEnd = normalizeWhitespace(haystack.slice(clusterEnd)).trimEnd()
    const ellipsisPre = trimmedStart.length < CONTEXT_LENGTH ? '' : '…'
    const ellipsisPost = trimmedEnd.length < CONTEXT_LENGTH ? '' : '…'
    const pre = `${ellipsisPre}${trimmedStart.slice(-CONTEXT_LENGTH)}`
    const post = `${trimmedEnd.slice(0, CONTEXT_LENGTH)}${ellipsisPost}`
    const segments = []
    let cursor = clusterStart
    for (const o of matched) {
        if (o.start > cursor) {
            const gap = normalizeWhitespace(haystack.slice(cursor, o.start))
            if (gap) segments.push({ text: gap, emphasized: false })
        }
        segments.push({ text: normalizeWhitespace(haystack.slice(o.start, o.end)), emphasized: true })
        cursor = o.end
    }
    const match = normalizeWhitespace(haystack.slice(clusterStart, clusterEnd))
    return { pre, match, post, segments }
}

// Calibre-parity nearby-words mode (#4560): matches places where all of the
// query's distinct whole words occur within `nearbyWords` words of each other.
// Distance is measured in words (not characters) and comes from the option, not
// from the query string, so trailing numbers stay literal search words.
const nearbyWordsSearch = function* (strs, query, options = {}) {
    const { locales = 'en', sensitivity = 'base', nearbyWords = 10 } = options
    const queryWords = []
    for (const w of query.split(/\s+/).filter(Boolean)) if (!queryWords.includes(w)) queryWords.push(w)
    if (queryWords.length < 2) {
        const err = new Error('Nearby words search needs at least two words')
        err.code = 'NEARBY_NEEDS_TWO_WORDS'
        throw err
    }
    let segmenter, collator
    try {
        segmenter = new Intl.Segmenter(locales, { usage: 'search', granularity: 'word' })
        collator = new Intl.Collator(locales, { sensitivity })
    } catch (e) {
        console.warn(e)
        segmenter = new Intl.Segmenter('en', { usage: 'search', granularity: 'word' })
        collator = new Intl.Collator('en', { sensitivity })
    }
    const haystack = strs.join('')
    const cum = buildCum(strs)
    const K = queryWords.length

    // Word occurrences of any query word, tagged with a global word index.
    const occ = []
    let wordIndex = -1
    for (const seg of segmenter.segment(haystack)) {
        if (!seg.isWordLike) continue
        wordIndex++
        for (let q = 0; q < K; q++) {
            if (collator.compare(queryWords[q], seg.segment) === 0) {
                occ.push({ wordIndex, qIdx: q, start: seg.index, end: seg.index + seg.segment.length })
                break
            }
        }
    }

    // Smallest window covering all K distinct query words, two-pointer scan.
    const have = new Array(K).fill(0)
    let distinct = 0
    let lo = 0
    const windows = []
    for (let hi = 0; hi < occ.length; hi++) {
        if (have[occ[hi].qIdx]++ === 0) distinct++
        let minimal = null
        while (distinct === K) {
            minimal = { lo, hi }
            if (--have[occ[lo].qIdx] === 0) distinct--
            lo++
        }
        if (minimal && occ[minimal.hi].wordIndex - occ[minimal.lo].wordIndex <= nearbyWords)
            windows.push(minimal)
    }

    // One cluster per window; suppress windows overlapping an already-emitted one.
    let lastHi = -1
    for (const w of windows) {
        if (w.lo <= lastHi) continue
        lastHi = w.hi
        const matched = occ.slice(w.lo, w.hi + 1)
        const excerpt = makeNearbyExcerpt(haystack, matched)
        const range = rangeFromFlat(cum, matched[0].start, matched[matched.length - 1].end)
        const subRanges = matched.map(o => rangeFromFlat(cum, o.start, o.end))
        yield { range, excerpt, subRanges }
    }
}

export const search = (strs, query, options) => {
    const { mode } = options
    if (mode === 'regex') return regexSearch(strs, query, options)
    if (mode === 'nearby-words') return nearbyWordsSearch(strs, query, options)
    const { granularity = 'grapheme', sensitivity = 'base' } = options
    if (!Intl?.Segmenter || granularity === 'grapheme'
    && (sensitivity === 'variant' || sensitivity === 'accent'))
        return simpleSearch(strs, query, options)
    return segmenterSearch(strs, query, options)
}

export const searchMatcher = (textWalker, opts) => {
    const { defaultLocale, matchCase, matchDiacritics, matchWholeWords, mode, nearbyWords, acceptNode } = opts
    const effectiveMode = mode ?? (matchWholeWords ? 'whole-words' : 'contains')
    return function* (doc, query) {
        const iter = textWalker(doc, function* (strs, makeRange) {
            for (const result of search(strs, query, {
                mode: effectiveMode,
                nearbyWords,
                matchCase,
                locales: doc.body.lang || doc.documentElement.lang || defaultLocale || 'en',
                granularity: effectiveMode === 'whole-words' ? 'word' : 'grapheme',
                sensitivity: matchDiacritics && matchCase ? 'variant'
                : matchDiacritics && !matchCase ? 'accent'
                : !matchDiacritics && matchCase ? 'case'
                : 'base',
            })) {
                const { startIndex, startOffset, endIndex, endOffset } = result.range
                result.range = makeRange(startIndex, startOffset, endIndex, endOffset)
                if (result.subRanges) result.subRanges = result.subRanges.map(
                    r => makeRange(r.startIndex, r.startOffset, r.endIndex, r.endOffset))
                yield result
            }
        }, acceptNode)
        for (const result of iter) yield result
    }
}
