// @ts-nocheck
const wait = ms => new Promise(resolve => setTimeout(resolve, ms))

// A microtask (or rAF alone) resumes before paint. Give input and rendering a
// turn between the style, host-load, and pagination phases of a chapter load.
const yieldToFrame = () => document.hidden ? wait(0)
    : new Promise(resolve => requestAnimationFrame(() => setTimeout(resolve, 0)))

// WebKit before Safari 17 (iOS <= 16) resolves the `document.fonts.ready`
// promise synchronously while purging still-loading `@font-face`s during a
// style resolver rebuild (CSSFontFaceSet::purge). Script execution is
// disallowed in the middle of style resolution, so the resolution trips a
// release assert and kills the WebContent process (DeferredPromise::callFunction
// under CSSFontSelector::buildStarted in the .ips crash log).
// The deferring guard only landed in WebKit 616 (Safari/iOS 17), the same
// release that shipped URL.canParse. The JS promise is created lazily on
// first access of `.ready`, so on affected engines never touch it and poll
// `fonts.status` instead.
export const fontsReady = doc => {
    const fonts = doc?.fonts
    const win = doc?.defaultView
    if (!fonts || !win) return Promise.resolve()
    const buggyWebKit = win.navigator?.userAgent?.includes('AppleWebKit/605')
        && typeof win.URL?.canParse !== 'function'
    if (!buggyWebKit) return fonts.ready
    return new Promise(resolve => {
        const poll = () => fonts.status === 'loaded'
            ? resolve() : win.setTimeout(poll, 100)
        poll()
    })
}

const debounce = (f, wait, immediate) => {
    let timeout
    return (...args) => {
        const later = () => {
            timeout = null
            if (!immediate) f(...args)
        }
        const callNow = immediate && !timeout
        if (timeout) clearTimeout(timeout)
        timeout = setTimeout(later, wait)
        if (callNow) f(...args)
    }
}

// How long a continuous run of scrolled-mode scroll events may last before a
// relocate is forced mid-scroll, so reading progress keeps updating while the
// scrolling never pauses (readest#5635).
const SCROLL_RELOCATE_MAX_WAIT = 1000

// Transforms ALL children of the container so multi-view layouts
// animate as a unified whole. Extra elements (e.g. background) are
// also transformed so they slide in sync with the content.
const cssAnimateScroll = (element, scrollProp, startValue, endValue, duration, extraElements = []) => new Promise(resolve => {
    if (document.hidden) {
        element[scrollProp] = endValue
        return resolve()
    }

    const children = [...element.children]
    if (!children.length) {
        element[scrollProp] = endValue
        return resolve()
    }

    const allElements = [...children, ...extraElements]
    const isHorizontal = scrollProp === 'scrollLeft'
    const delta = endValue - startValue
    const transformProp = isHorizontal ? 'translateX' : 'translateY'

    // Prepare all elements for animation
    for (const el of allElements) {
        el.style.willChange = 'transform'
        el.style.transform = `${transformProp}(0px)`
        el.style.transition = 'none'
    }

    // Force reflow to apply initial state
    element.getBoundingClientRect()

    // Start animation on all elements
    for (const el of allElements) {
        el.style.transition = `transform ${duration}ms cubic-bezier(0.25, 0.46, 0.45, 0.94)`
        el.style.transform = `${transformProp}(${-delta}px)`
    }

    let resolved = false
    const cleanup = () => {
        if (resolved) return
        resolved = true

        for (const el of allElements) {
            el.style.willChange = ''
            el.style.transform = ''
            el.style.transition = ''
        }

        // Apply final scroll position
        element[scrollProp] = endValue
        // Translating the children shrank the container's scrollable overflow
        // by the animated distance, and WebKit (iOS 18) still reports that
        // shrunken extent when the transforms are cleared in this same task —
        // so the assignment above is clamped. Mid-book the clamp lands far
        // beyond the target and is invisible; on the last page of the book,
        // where the target *is* the maximum scroll offset, it lands a full page
        // short and the page visibly snaps back (readest#5663).
        if (Math.abs(element[scrollProp] - endValue) > 0.5) {
            // Forcing a layout here is not enough — the extent it reports is
            // restored, but the scroll clamp still uses the stale one for the
            // rest of the task. The next frame is past the compositor's
            // animation teardown, so re-apply there.
            requestAnimationFrame(() => {
                element[scrollProp] = endValue
                resolve()
            })
            return
        }
        resolve()
    }

    // Listen for transition end on the first child
    const first = children[0]
    const onTransitionEnd = (e) => {
        if (e.target === first && e.propertyName === 'transform') {
            first.removeEventListener('transitionend', onTransitionEnd)
            cleanup()
        }
    }
    first.addEventListener('transitionend', onTransitionEnd)

    // Fallback timeout in case transitionend doesn't fire
    setTimeout(cleanup, duration + 50)
})

// Two-phase page-turn slide for vertical (vertical-rl/lr) paginated books.
// Their pages read horizontally but CSS fragmentation stacks them along the
// vertical scroll axis, so the outgoing and incoming page can never be on
// screen side by side (readest#624). Instead the outgoing page exits
// horizontally along the page progression, the scroll offset jumps while the
// viewport shows only the page background, and the incoming page follows in
// from the opposite edge. `startX` continues from a finger drag already in
// progress; `isStale` lets a newer turn supersede this one: when it reports
// true, this animation stops touching the DOM.
const slideTurnAnimation = (element, scrollProp, endValue, exitSign, width, duration, isStale, onSwap, startX = 0) => new Promise(resolve => {
    const children = [...element.children]
    if (document.hidden || !children.length) {
        element[scrollProp] = endValue
        return resolve()
    }
    const half = duration / 2
    const exitTarget = exitSign * width
    // Scale the exit by the distance the drag already covered so a released
    // drag continues at the same pace instead of restarting from rest.
    const exitDuration = Math.max(16, half * Math.min(1, Math.abs(exitTarget - startX) / width))
    const setAll = (transition, transform) => {
        for (const el of children) {
            el.style.transition = transition
            el.style.transform = transform
        }
    }
    for (const el of children) el.style.willChange = 'transform'
    // Phase 1: the outgoing page accelerates off-screen.
    setAll('none', `translateX(${startX}px)`)
    element.getBoundingClientRect()
    setAll(`transform ${exitDuration}ms cubic-bezier(0.55, 0, 1, 0.45)`, `translateX(${exitTarget}px)`)
    setTimeout(() => {
        if (isStale()) return resolve()
        // Midpoint: swap pages while everything is off-screen.
        element[scrollProp] = endValue
        onSwap?.()
        setAll('none', `translateX(${-exitSign * width}px)`)
        element.getBoundingClientRect()
        // Phase 2: the incoming page decelerates into place.
        setAll(`transform ${half}ms cubic-bezier(0, 0.55, 0.45, 1)`, 'translateX(0px)')
        setTimeout(() => {
            if (isStale()) return resolve()
            for (const el of children) {
                el.style.willChange = ''
                el.style.transition = ''
                el.style.transform = ''
            }
            resolve()
        }, half + 20)
    }, exitDuration + 10)
})

// Layered page-turn styles (readest#555). The `slide` and `curl` turn styles
// need the outgoing and incoming page on screen at once as separate layers,
// which the rigid column strip inside one iframe cannot provide. The View
// Transitions API can: the browser rasterizes the outgoing page (overlays and
// annotations included) as a snapshot that animates over the live, stationary
// incoming page — an Apple Books style slide or curl. The choreography lives
// in a document-level stylesheet because the ::view-transition pseudo tree
// attaches to the document root, not to the paginator's shadow root.
const VIEW_TRANSITION_CLASSES = [
    'foliate-vt', 'foliate-vt-slide', 'foliate-vt-curl',
    'foliate-vt-scrub',
    'foliate-vt-forward', 'foliate-vt-backward',
    'foliate-vt-left', 'foliate-vt-right',
    'foliate-vt-eat-left', 'foliate-vt-eat-right',
]

const RELEASE_VELOCITY_WINDOW_MS = 90
const RELEASE_PAUSE_THRESHOLD_MS = 80
const SLIDE_RELEASE_PROJECTION_MS = 240
// Fraction of a page a drag-follow swipe must cover for the release to
// commit the turn on displacement alone, when the finger is too slow to
// register as a flick.
const DRAG_TURN_DISTANCE_RATIO = 0.06
const LAYERED_EDGE_REGION = 0.18
const LAYERED_EARLY_CLAIM_PX = 6
const LAYERED_EARLY_SAMPLE_INTERVAL_MS = 80
const LAYERED_VERTICAL_REJECT_PX = 8
const LAYERED_FALLBACK_CLAIM_PX = 24
const LAYERED_FALLBACK_DOMINANCE = 1.5

const updateReleaseSample = (state, distance, time) => {
    const previous = state.releaseSamples.at(-1)
    if (!previous || distance !== previous.distance) state.lastMovementTime = time
    if (previous?.time === time) previous.distance = distance
    else state.releaseSamples.push({ distance, time })

    const cutoff = time - RELEASE_VELOCITY_WINDOW_MS
    while (state.releaseSamples.length > 2
        && state.releaseSamples[1].time < cutoff) state.releaseSamples.shift()
}

const getReleaseVelocity = state => {
    const latest = state.releaseSamples.at(-1)
    if (!latest || latest.time - state.lastMovementTime > RELEASE_PAUSE_THRESHOLD_MS) return 0

    const cutoff = latest.time - RELEASE_VELOCITY_WINDOW_MS
    const before = state.releaseSamples[0]
    const after = state.releaseSamples.find(sample => sample.time >= cutoff)
    if (!before || !after) return 0

    const startTime = Math.max(cutoff, before.time)
    if (latest.time <= startTime) return 0
    const interval = after.time - before.time
    const startDistance = interval > 0 && startTime > before.time
        ? before.distance
            + (after.distance - before.distance) * (startTime - before.time) / interval
        : after.distance
    return (latest.distance - startDistance) / (latest.time - startTime)
}

// Release speed controls the remaining settle rate for both layered styles.
// Slide can carry more momentum than the heavier curl.
const LAYERED_SETTLE_CONFIG = {
    slide: { minSpeed: 0.2, maxSpeed: 1, maxRate: 2 },
    curl: { minSpeed: 0.3, maxSpeed: 1.5, maxRate: 1.5 },
}

const layeredSettlePlaybackRate = (style, speed) => {
    const config = LAYERED_SETTLE_CONFIG[style]
    if (!config || !(speed > config.minSpeed)) return 1
    const { minSpeed, maxSpeed, maxRate } = config
    const amount = Math.min(1, (speed - minSpeed) / (maxSpeed - minSpeed))
    return 1 + amount * (maxRate - 1)
}

const updatePlaybackRate = (animation, rate) => {
    if (rate === 1) return
    try {
        animation.updatePlaybackRate(rate)
        return
    } catch { /* unsupported for this UA animation */ }
    try { animation.playbackRate = rate } catch { /* unsupported */ }
}

const injectViewTransitionStyles = () => {
    const id = 'foliate-view-transition-styles'
    if (document.getElementById(id)) return
    const style = document.createElement('style')
    style.id = id
    style.textContent = `
    .foliate-vt::view-transition {
        pointer-events: none;
    }
    /* Only the page turn animates; keep the root snapshot inert. */
    .foliate-vt::view-transition-old(root),
    .foliate-vt::view-transition-new(root) {
        animation: none;
    }
    /* The turn layers must OCCLUDE, not blend: the UA pairs old/new with
       mix-blend-mode: plus-lighter for its default cross-fade, which turns
       the still page ghostly under a moving page. Also back both layers
       with the page colour, since snapshots of textured themes or books
       without a background are transparent. */
    .foliate-vt::view-transition-old(foliate-turn),
    .foliate-vt::view-transition-new(foliate-turn) {
        animation: none;
        background: var(--foliate-vt-bg, Canvas);
        mix-blend-mode: normal;
    }
    /* Slide: the moving page travels over the still page with a soft edge
       shadow, like the Apple Books slide. Forward moves the outgoing
       snapshot out on top; backward brings the incoming snapshot in on
       top of the still outgoing page. */
    .foliate-vt-slide.foliate-vt-forward::view-transition-old(foliate-turn) {
        z-index: 1;
        animation: foliate-turn-slide-out-left 300ms cubic-bezier(0.25, 0.46, 0.45, 0.94) both;
        box-shadow: 0 0 24px rgba(0, 0, 0, 0.35);
    }
    .foliate-vt-slide.foliate-vt-forward.foliate-vt-right::view-transition-old(foliate-turn) {
        animation-name: foliate-turn-slide-out-right;
    }
    .foliate-vt-slide.foliate-vt-backward::view-transition-new(foliate-turn) {
        animation: foliate-turn-slide-in-left 300ms cubic-bezier(0.25, 0.46, 0.45, 0.94) both;
        box-shadow: 0 0 24px rgba(0, 0, 0, 0.35);
    }
    .foliate-vt-slide.foliate-vt-backward.foliate-vt-right::view-transition-new(foliate-turn) {
        animation-name: foliate-turn-slide-in-right;
    }
    /* Finger-tracked turns map distance directly to animation time. Declare
       linear timing at the CSS source because some Android WebViews expose
       UA pseudo animations but reject KeyframeEffect.updateTiming(). */
    .foliate-vt-scrub::view-transition-old(foliate-turn),
    .foliate-vt-scrub::view-transition-new(foliate-turn) {
        animation-timing-function: linear !important;
    }
    @keyframes foliate-turn-slide-out-left { to { transform: translateX(-100%); } }
    @keyframes foliate-turn-slide-out-right { to { transform: translateX(100%); } }
    @keyframes foliate-turn-slide-in-left { from { transform: translateX(-100%); } }
    @keyframes foliate-turn-slide-in-right { from { transform: translateX(100%); } }
    /* Curl: a fold line travels across the page, peeling it off the still
       page underneath like the Apple Books / Kindle curl. The fold is an
       oversized gradient mask slid across the OLD snapshot (mask-position is
       animatable; gradients themselves are not), so the page dissolves over
       a soft band at the traveling edge — the lifted-page falloff. Chrome
       paints masks on the static old snapshot but not on the live new layer,
       so backward turns also choreograph the old page: it recedes from the
       spine side, which reads the same as the incoming page unfolding.
       Filters and shadows are applied before masking and would be cut off
       with the page, hence the soft edge. A flat snapshot cannot mesh-bend
       like a native curl; this is the closest two-layer approximation. */
    /* The curl consumes the old page along a CURVED fold: a transparent
       disc grows out of the page's outer-bottom corner (the corner a reader
       lifts), so the fold edge is an arc sweeping across the page toward
       the spine — the bent-page line of a corner curl. Backward turns grow
       the arc from the spine-side corner instead, receding the old page so
       the previous page appears to unfold (eat side precomputed on the
       root). The fold edge is an animated gradient STOP (registered custom
       property) re-rasterized each frame against the element box:
       mask-position/mask-size animations paint unreliably on
       view-transition pseudos. The 6% band is the lifted-page falloff. */
    @property --foliate-fold {
        syntax: '<percentage>';
        inherits: false;
        initial-value: 0%;
    }
    .foliate-vt-curl.foliate-vt-eat-right::view-transition-old(foliate-turn) {
        z-index: 1;
        -webkit-mask-image: radial-gradient(circle at 108% 108%, transparent calc(var(--foliate-fold) - 6%), black var(--foliate-fold));
        mask-image: radial-gradient(circle at 108% 108%, transparent calc(var(--foliate-fold) - 6%), black var(--foliate-fold));
        animation: foliate-turn-curl-fold 450ms cubic-bezier(0.3, 0.1, 0.4, 1) both;
    }
    .foliate-vt-curl.foliate-vt-eat-left::view-transition-old(foliate-turn) {
        z-index: 1;
        -webkit-mask-image: radial-gradient(circle at -8% 108%, transparent calc(var(--foliate-fold) - 6%), black var(--foliate-fold));
        mask-image: radial-gradient(circle at -8% 108%, transparent calc(var(--foliate-fold) - 6%), black var(--foliate-fold));
        animation: foliate-turn-curl-fold 450ms cubic-bezier(0.3, 0.1, 0.4, 1) both;
    }
    .foliate-vt-curl::view-transition-new(foliate-turn) {
        animation: none;
    }
    @keyframes foliate-turn-curl-fold {
        from { --foliate-fold: 0%; }
        to { --foliate-fold: 118%; }
    }
    `
    document.head.append(style)
}

const lerp = (min, max, x) => x * (max - min) + min
const easeOutQuad = x => 1 - (1 - x) * (1 - x)
// rAF animation of a scalar (used for the native scroll offset). Unlike the
// CSS-transform animate, this never composites the whole section as a
// single layer, so it doesn't block when the section exceeds the GPU texture
// limit — it just changes scroll offset each frame (incremental/tiled).
const rafAnimateScroll = (a, b, duration, ease, render) => new Promise(resolve => {
    let start
    const step = now => {
        if (document.hidden) {
            render(lerp(a, b, 1))
            return resolve()
        }
        start ??= now
        const fraction = Math.min(1, (now - start) / duration)
        render(lerp(a, b, ease(fraction)))
        if (fraction < 1) requestAnimationFrame(step)
        else resolve()
    }
    if (document.hidden) {
        render(lerp(a, b, 1))
        return resolve()
    }
    requestAnimationFrame(step)
})

// A CSS-transform page-turn must composite the whole section as one layer. Once
// that layer is past the GPU texture limit (large sections; worse at high DPR on
// Android) Blink blocks the UI for ~1s preparing it before the turn snaps. Above
// this accumulated rendered-view size, animate the native scroll offset instead.
const RAF_ANIMATE_SCROLL_THRESHOLD = 20000

// collapsed range doesn't return client rects sometimes (or always?)
// try make get a non-collapsed range or element
const uncollapse = range => {
    if (!range?.collapsed) return range
    const { endOffset, endContainer } = range
    if (endContainer.nodeType === 1) {
        const node = endContainer.childNodes[endOffset]
        if (node?.nodeType === 1) return node
        return endContainer
    }
    if (endOffset + 1 < endContainer.length) range.setEnd(endContainer, endOffset + 1)
    else if (endOffset > 1) range.setStart(endContainer, endOffset - 1)
    else return endContainer.parentNode
    return range
}

const makeRange = (doc, node, start, end = start) => {
    const range = doc.createRange()
    range.setStart(node, start)
    range.setEnd(node, end)
    return range
}

// use binary search to find an offset value in a text node
const bisectNode = (doc, node, cb, start = 0, end = node.nodeValue.length) => {
    if (end - start === 1) {
        const result = cb(makeRange(doc, node, start), makeRange(doc, node, end))
        return result < 0 ? start : end
    }
    const mid = Math.floor(start + (end - start) / 2)
    const result = cb(makeRange(doc, node, start, mid), makeRange(doc, node, mid, end))
    return result < 0 ? bisectNode(doc, node, cb, start, mid)
        : result > 0 ? bisectNode(doc, node, cb, mid, end) : mid
}

const { SHOW_ELEMENT, SHOW_TEXT, SHOW_CDATA_SECTION,
    FILTER_ACCEPT, FILTER_REJECT, FILTER_SKIP } = NodeFilter

const filter = SHOW_ELEMENT | SHOW_TEXT | SHOW_CDATA_SECTION

// needed cause there seems to be a bug in `getBoundingClientRect()` in Firefox
// where it fails to include rects that have zero width and non-zero height
// (CSSOM spec says "rectangles [...] of which the height or width is not zero")
// which makes the visible range include an extra space at column boundaries
const getBoundingClientRect = target => {
    let top = Infinity, right = -Infinity, left = Infinity, bottom = -Infinity
    for (const rect of target.getClientRects()) {
        left = Math.min(left, rect.left)
        top = Math.min(top, rect.top)
        right = Math.max(right, rect.right)
        bottom = Math.max(bottom, rect.bottom)
    }
    return new DOMRect(left, top, right - left, bottom - top)
}

const getVisibleRange = (doc, start, end, mapRect) => {
    // A resize/scroll callback can fire after the view's document has been
    // torn down (e.g. during teardown, or while an async section load is still
    // settling); there is nothing to measure without a body.
    if (!doc?.body) return
    // first get all visible nodes
    const acceptNode = node => {
        const name = node.localName?.toLowerCase()
        // ignore all scripts, styles, and their children
        if (name === 'script' || name === 'style') return FILTER_REJECT
        // ignore cfi-inert nodes (e.g. injected a11y skip-links) and their
        // subtree: they are invisible to CFI, so anchoring the visible range on
        // one yields a degenerate CFI and can crash `fromRange` when such a node
        // is the only child of its parent (content-less background sections).
        if (node.nodeType === 1 && node.hasAttribute?.('cfi-inert')) return FILTER_REJECT
        if (node.nodeType === 1) {
            const { left, right } = mapRect(node.getBoundingClientRect())
            if (left === 0 && right === 0) return FILTER_REJECT
            // no need to check child nodes if it's completely out of view
            if (right < start || left > end) return FILTER_REJECT
            // elements must be completely in view to be considered visible
            // because you can't specify offsets for elements
            if (left >= start && right <= end) return FILTER_ACCEPT
            // TODO: it should probably allow elements that do not contain text
            // because they can exceed the whole viewport in both directions
            // especially in scrolled mode
        } else {
            // ignore empty text nodes
            if (!node.nodeValue?.trim()) return FILTER_SKIP
            // create range to get rect
            const range = doc.createRange()
            range.selectNodeContents(node)
            const { left, right } = mapRect(range.getBoundingClientRect())
            // it's visible if any part of it is in view
            if (left === 0 && right === 0) return FILTER_REJECT
            if (right >= start && left <= end) return FILTER_ACCEPT
        }
        return FILTER_SKIP
    }
    const walker = doc.createTreeWalker(doc.body, filter, { acceptNode })
    const nodes = []
    for (let node = walker.nextNode(); node; node = walker.nextNode())
        nodes.push(node)

    // we're only interested in the first and last visible nodes
    const from = nodes[0] ?? doc.body
    const to = nodes[nodes.length - 1] ?? from

    // find the offset at which visibility changes
    const startOffset = from.nodeType === 1 ? 0
        : bisectNode(doc, from, (a, b) => {
            const p = mapRect(getBoundingClientRect(a))
            const q = mapRect(getBoundingClientRect(b))
            if (p.right < start && q.left > start) return 0
            return q.left > start ? -1 : 1
        })
    const endOffset = to.nodeType === 1 ? 0
        : bisectNode(doc, to, (a, b) => {
            const p = mapRect(getBoundingClientRect(a))
            const q = mapRect(getBoundingClientRect(b))
            if (p.right < end && q.left > end) return 0
            return q.left > end ? -1 : 1
        })

    const range = doc.createRange()
    range.setStart(from, startOffset)
    range.setEnd(to, endOffset)
    return range
}

const selectionIsBackward = sel => {
    const range = document.createRange()
    range.setStart(sel.anchorNode, sel.anchorOffset)
    range.setEnd(sel.focusNode, sel.focusOffset)
    return range.collapsed
}

const setSelectionTo = (target, collapse) => {
    let range
    if (target.startContainer) range = target.cloneRange()
    else if (target.nodeType) {
        range = document.createRange()
        range.selectNode(target)
    }
    if (range) {
        const sel = range.startContainer.ownerDocument?.defaultView.getSelection()
        if (sel) {
            sel.removeAllRanges()
            if (collapse === -1) range.collapse(true)
            else if (collapse === 1) range.collapse()
            sel.addRange(range)
        }
    }
}

// Whether a view's bounding rect overlaps the visible region of its container.
// Used by #syncA11y to mark only the pre-loaded views that lie outside the
// viewport as `aria-hidden`. Views still visible to sighted users (e.g. the
// right column in a dual-page spread that belongs to a different section
// than the left column) stay exposed to assistive tech.
// See readest/readest#4243 and readest/readest#4259.
export const isViewVisibleInContainer = (viewRect, containerRect) =>
    viewRect.right > containerRect.left
    && viewRect.left < containerRect.right
    && viewRect.bottom > containerRect.top
    && viewRect.top < containerRect.bottom

export const getDirection = doc => {
    const { defaultView } = doc
    // A view's iframe document can be blank/detached while a section loads or
    // the view is torn down, leaving body null; getComputedStyle(null) then
    // throws "parameter 1 is not of type 'Element'" (READEST-2X). Fall back to
    // horizontal-ltr until real content is present.
    if (!defaultView || !doc.body) return { vertical: false, rtl: false }
    let { writingMode, direction } = defaultView.getComputedStyle(doc.body)
    // Some EPUBs set writing-mode on the first child of body instead of body itself
    if (!writingMode || writingMode === 'horizontal-tb') {
        const firstChild = doc.body.querySelector(':scope > :not([cfi-inert])')
        if (firstChild) {
            const childStyle = defaultView.getComputedStyle(firstChild)
            if (childStyle.writingMode === 'vertical-rl'
                || childStyle.writingMode === 'vertical-lr') {
                writingMode = childStyle.writingMode
            }
        }
    }
    const vertical = writingMode === 'vertical-rl'
        || writingMode === 'vertical-lr'
    // `vertical-rl` (Japanese/Chinese vertical) advances columns right-to-left
    // even though its computed `direction` stays `ltr`, so the writing mode
    // itself marks it RTL and page turns follow the horizontal-rtl convention
    // (readest#624). Mirrors getDirection in the app's libs/document.ts.
    const rtl = writingMode === 'vertical-rl'
        || doc.body.dir === 'rtl'
        || direction === 'rtl'
        || doc.documentElement.dir === 'rtl'
    return { vertical, rtl }
}

// The spine's `page-progression-direction` is a publication-wide declaration,
// so it outranks the direction of any single content document: an LTR colophon
// in a Japanese book still pages right-to-left. Only `ltr` and `rtl` are
// binding — an absent attribute or the `default` keyword leaves the choice to
// the Reading System, which falls back to the document.
export const getPageProgressionRTL = (bookDir, documentRTL) =>
    bookDir === 'rtl' ? true : bookDir === 'ltr' ? false : documentRTL

const getBackground = doc => {
    // Same blank/detached-document guard as getDirection (READEST-2X).
    if (!doc.defaultView || !doc.body) return ''
    const bodyStyle = doc.defaultView.getComputedStyle(doc.body)
    return bodyStyle.backgroundColor === 'rgba(0, 0, 0, 0)'
        && bodyStyle.backgroundImage === 'none'
        ? doc.defaultView.getComputedStyle(doc.documentElement).background
        : bodyStyle.background
}

// Compute the background segments for paginated mode. Each rendered view yields
// one segment positioned so it tracks its content on screen
// (segStart = inset + viewOffset - scrollPos). Because the paginator rebuilds
// these on every scroll, the backgrounds stay glued to the content while the
// user drags a swipe; when two sections with different backgrounds are both on
// screen the seam falls on the real content boundary instead of one flat colour
// spanning the viewport.
//
// Each segment is clamped to the content area [containerStart, containerEnd] so
// a coloured page stays inside its own column and never bleeds into the outer
// margin gutters (the --_outer-min tracks that keep the left/right margins in
// step with the centre gap). Otherwise a body-coloured page would spill its
// colour into the outer gutter while an adjacent transparent/image page did not,
// shifting the spread off-centre (~250px wide on a desktop, readest#4394). In
// single-column mode the gutters are zero, so the clamp still fills the viewport
// edge to edge. `views` is the sorted list of { size, bg } with bg already
// resolved ('' = transparent → no segment).
export const computeBackgroundSegments = (views, scrollPos, bgSize, inset, containerSize) => {
    const containerStart = inset
    const containerEnd = inset + containerSize
    const segments = []
    let offset = 0
    for (const view of views) {
        const segStart = inset + offset - scrollPos
        const segEnd = segStart + view.size
        offset += view.size
        if (segEnd <= 0 || segStart >= bgSize) continue // off screen
        if (!view.bg) continue // transparent → let the host/theme show through
        const start = Math.max(segStart, containerStart)
        const end = Math.min(segEnd, containerEnd)
        if (end <= start) continue // entirely in an outer gutter
        segments.push({ start, size: end - start, bg: view.bg })
    }
    return segments
}

// When a host background texture is active (mounted on the reader container as
// `.foliate-viewer::before`), a page whose own background is transparent must
// NOT paint a fill — an opaque fill would occlude the texture. Returns '' (no
// fill, so the texture shows through) for a transparent page under a texture,
// and the resolved colour otherwise. Shared by scrolled-mode view elements and
// paginated-mode segments so both modes treat textures identically (readest#4399).
export const textureAwareBackground = (resolved, hasTexture) => {
    // A page that paints an image (e.g. a cover set via body `background-image`)
    // is NOT transparent — it should occlude the texture, not be dropped. The
    // computed `background` shorthand always serializes the transparent
    // background-*color* first (`rgba(0, 0, 0, 0) url(...) ...`), so the
    // colour-prefix check below would otherwise misclassify a cover as
    // transparent and hide it behind the texture (verified on Android WebView).
    const hasImage = /\burl\(/i.test(resolved ?? '')
    const isTransparent = !hasImage && (!resolved
        || /^\s*(transparent|rgba\(0,\s*0,\s*0,\s*0\))/.test(resolved))
    return hasTexture && isTransparent ? '' : resolved
}

const makeMarginals = (length, part) => Array.from({ length }, () => {
    const div = document.createElement('div')
    const child = document.createElement('div')
    div.append(child)
    child.setAttribute('part', part)
    return div
})

const setStyles = (el, styles) => {
    // el is doc.documentElement, which is null while a view's document is blank
    // or detached mid-render/teardown (READEST-1H). Nothing to style then.
    if (!el) return
    const { style } = el
    for (const [k, v] of Object.entries(styles)) style.setProperty(k, v)
}

const setStylesImportant = (el, styles) => {
    if (!el) return
    const { style } = el
    for (const [k, v] of Object.entries(styles)) style.setProperty(k, v, 'important')
}

class View {
    #observer = new ResizeObserver(() => this.expand())
    #element = document.createElement('div')
    #iframe = document.createElement('iframe')
    #contentRange = document.createRange()
    #overlayer
    #vertical = false
    #rtl = false
    // The document's own inline direction, read before any of our overrides
    // touch it. `#rtl` is the book's page progression, which can disagree.
    #docDirection = 'ltr'
    // The document `load()` has taken `#docDirection` from. A re-render can
    // reach this view earlier, while the iframe holds a section whose load
    // event has not fired yet; see `render()`.
    #loadedDoc = null
    #directionStyle = null
    #column = true
    #size
    #columnCount = 1
    #layout = {}
    #contentPages = 0
    #bgImageSize = null
    fontReady = Promise.resolve()
    constructor({ container, onExpand }) {
        this.container = container
        this.onExpand = onExpand
        this.#iframe.setAttribute('part', 'filter')
        this.#element.append(this.#iframe)
        Object.assign(this.#element.style, {
            boxSizing: 'content-box',
            position: 'relative',
            overflow: 'hidden',
            flex: '0 0 auto',
            width: '100%', height: '100%',
            display: 'flex',
            justifyContent: 'flex-start',
            alignItems: 'center',
        })
        // Hidden but laid out, rather than `display: none`. A section's own
        // scripts run while the document loads, before this view has rendered,
        // and a display:none subtree has no boxes at all, so anything the book
        // measures then comes back zero. The IDPF `trees` sample sizes its
        // canvas backing store from the element's own width, so it ended up
        // 0x0 and drew nothing at all (readest/readest#6041). `visibility:
        // hidden` still paints nothing while giving those scripts real
        // geometry to measure.
        Object.assign(this.#iframe.style, {
            overflow: 'hidden',
            border: '0',
            visibility: 'hidden',
            width: '100%', height: '100%',
        })
        // `allow-scripts` is needed for events because of WebKit bug
        // https://bugs.webkit.org/show_bug.cgi?id=218086
        this.#iframe.setAttribute('sandbox', 'allow-same-origin allow-scripts')
        this.#iframe.setAttribute('scrolling', 'no')
    }
    get element() {
        return this.#element
    }
    get document() {
        return this.#iframe.contentDocument
    }
    get contentPages() {
        return this.#contentPages
    }
    async load(src, data, afterLoad, beforeRender, waitForIdle) {
        if (typeof src !== 'string') throw new Error(`${src} is not string`)
        return new Promise(resolve => {
            this.#iframe.addEventListener('load', async () => {
                // A background fetch can finish after a swipe has started.
                // Defer its style/layout work, not just the start of preloading.
                if (waitForIdle) await waitForIdle()
                const doc = this.document
                if (!doc?.documentElement || !doc.body) return resolve()
                await afterLoad?.(doc)
                await yieldToFrame()
                if (this.document !== doc) return resolve()

                this.#iframe.setAttribute('aria-label', doc.title)
                // it needs to be visible for Firefox to get computed style
                this.#iframe.style.visibility = 'visible'
                const { vertical, rtl } = getDirection(doc)
                this.docBackground = getBackground(doc)
                doc.body.style.background = 'none'
                // Resolve the body background image's natural size BEFORE the
                // first render so the scrolled-mode view is sized to fit it
                // from the start. Sizing it lazily — expanding only once the
                // image loads — grows the view *after* navigation has already
                // scrolled to it. On reopen that growth lands above the saved
                // position (e.g. a preloaded previous section's full-page
                // illustration) and, with no reliable cross-iframe scroll
                // anchoring on WebKit, drifts the viewport to the chapter
                // start. Awaiting a local EPUB resource here is near-instant.
                let bgRendered = false
                const bgUrl = this.docBackground
                    ?.match(/url\(["']?([^"')]+)["']?\)/)?.[1]
                if (bgUrl && !this.container.noBackground) {
                    const img = new Image()
                    let resolveWait
                    const waited = new Promise(res => { resolveWait = res })
                    img.onload = () => {
                        this.#bgImageSize = {
                            width: img.naturalWidth,
                            height: img.naturalHeight,
                        }
                        // If the image only resolves after this view has
                        // already rendered (slower than the bounded wait
                        // below), grow to fit it now — the original lazy path,
                        // kept as a fallback rather than the norm.
                        if (bgRendered && !this.#column) this.expand()
                        resolveWait()
                    }
                    // A missing or broken image just renders without the
                    // background, exactly as before.
                    img.onerror = () => resolveWait()
                    img.src = bgUrl
                    // Bound the wait so a missing, broken, or hung image (one
                    // that fires neither load nor error) can never block the
                    // section from rendering.
                    let timer
                    await Promise.race([
                        waited,
                        new Promise(res => { timer = setTimeout(res, 3000) }),
                    ])
                    clearTimeout(timer)
                    if (waitForIdle) await waitForIdle()
                }
                // Awaiting the background image yields control, so the view may
                // have been torn down or reloaded meanwhile — don't render into
                // a stale document.
                if (this.document !== doc) return resolve()
                this.#iframe.style.visibility = 'hidden'

                this.#vertical = vertical
                this.#rtl = rtl
                this.#docDirection =
                    doc.defaultView.getComputedStyle(doc.documentElement).direction === 'rtl'
                        ? 'rtl' : 'ltr'
                this.#loadedDoc = doc

                this.#contentRange.selectNodeContents(doc.body)
                const layout = beforeRender?.({ vertical, rtl })
                this.#iframe.style.visibility = 'visible'
                this.render(layout)
                bgRendered = true
                this.#observer.observe(doc.body)

                // the resize observer above doesn't work in Firefox
                // (see https://bugzilla.mozilla.org/show_bug.cgi?id=1832939)
                // until the bug is fixed we can at least account for font load
                this.fontReady = fontsReady(doc).then(() => this.expand())

                resolve()
            }, { once: true })
            if (data) {
                this.#iframe.srcdoc = data
            } else {
                this.#iframe.src = src
            }
        })
    }
    render(layout) {
        if (!layout || !this.document?.documentElement) return
        // A resize or an attribute change re-renders every view that already
        // has a document, and the iframe reports the incoming section as its
        // document as soon as it commits — well before the load event where
        // `#docDirection` is taken. Rendering it there would stamp the
        // progression override onto a document whose own direction has not
        // been read yet, and that read would then come back as the override
        // itself: the view would look like it already agrees with the book,
        // the override would be dropped, and the section's columns would run
        // against the scroll for good. The load handler renders it anyway.
        if (this.document !== this.#loadedDoc) return
        if (layout.rtl != null) this.#rtl = layout.rtl
        this.#column = layout.flow !== 'scrolled'
        this.#layout = layout
        // Column boxes are laid out along the multi-column container's inline
        // direction, so a section whose own direction disagrees with the book's
        // page progression would run its columns against the scroll and open on
        // its last page. Give the multicol box the progression instead, and hand
        // the document's own direction back to the content. Vertical writing
        // paginates along `scrollTop` with the host grid left alone, and there
        // `direction` picks the line-stacking axis rather than the column order,
        // so leave it — as scrolled flow, which has no columns to order, does.
        this.#setProgressionDirection(
            this.#column && !this.#vertical ? this.#rtl ? 'rtl' : 'ltr' : null)
        if (this.#column) this.columnize(layout)
        else this.scrolled(layout)
    }
    scrolled({ width, height, marginTop, marginRight, marginBottom, marginLeft, gap, columnWidth }) {
        const vertical = this.#vertical
        const doc = this.document
        setStylesImportant(doc.documentElement, {
            'box-sizing': 'border-box',
            'column-width': 'auto',
            'height': 'auto',
            'width': 'auto',
        })
        const availableWidth = Math.trunc(width - marginLeft - marginRight)
        const availableHeight = Math.trunc(height - marginTop - marginBottom)
        const sidePaddingLeft = marginLeft / 2 + gap / 2
        const sidePaddingRight = marginRight / 2 + gap / 2
        setStyles(doc.documentElement, {
            'padding': vertical
                ? `${marginTop * 1.5}px 0px ${marginBottom * 1.5}px 0px`
                : `0px ${sidePaddingRight}px 0px ${sidePaddingLeft}px`,
            '--page-margin-top': `${vertical ? marginTop * 1.5 : marginTop}px`,
            '--page-margin-right': `${vertical ? marginRight : sidePaddingRight}px`,
            '--page-margin-bottom': `${vertical ? marginBottom * 1.5 : marginBottom}px`,
            '--page-margin-left': `${vertical ? marginLeft : sidePaddingLeft}px`,
            '--full-width': `${Math.trunc(width)}`,
            '--full-height': `${Math.trunc(height)}`,
            '--available-width': `${availableWidth}`,
            '--available-height': `${availableHeight}`,
        })
        setStylesImportant(doc.body, {
            [vertical ? 'max-height' : 'max-width']: `${columnWidth}px`,
            'margin': 'auto',
            // Prevent position:absolute/fixed on body from coupling its
            // size to the iframe, which causes diverging expand() loops
            'position': 'static',
        })
        this.setImageSize(availableWidth, availableHeight)
        this.expand()
    }
    // In an HTML document the principal writing mode — the one the root
    // element's own box uses, and with it the order of its column boxes — is
    // taken from `body`, not from the root (CSS Writing Modes §8.1). So the
    // progression has to be written onto `body`, which is also where the text
    // direction lives; a rule one level down gives the content back the
    // direction the book authored, leaving elements that declare one alone.
    #setProgressionDirection(direction) {
        const doc = this.document
        if (!doc?.head) return
        if (!direction || direction === this.#docDirection) {
            doc.documentElement.style.removeProperty('direction')
            doc.body?.style.removeProperty('direction')
            this.#directionStyle?.remove()
            this.#directionStyle = null
            return
        }
        setStylesImportant(doc.documentElement, { 'direction': direction })
        if (doc.body) setStylesImportant(doc.body, { 'direction': direction })
        // The view outlives its documents, so a style element kept from the
        // previous section is detached by the time this one renders.
        if (this.#directionStyle?.ownerDocument !== doc) {
            this.#directionStyle = doc.createElement('style')
            doc.head.append(this.#directionStyle)
        }
        this.#directionStyle.textContent =
            `body > *:not([dir]) { direction: ${this.#docDirection}; }`
    }
    columnize({ width, height, marginTop, marginRight, marginBottom, marginLeft, gap, columnWidth, columnCount, columnGap }) {
        const vertical = this.#vertical
        this.#size = vertical ? height : width
        this.#columnCount = columnCount || 1

        const doc = this.document
        const {
            columnGap: horizontalColumnGap, sidePaddingLeft, sidePaddingRight,
            columnWidth: columnWidthHint, availableWidth: horizontalAvailableWidth,
        } = getHorizontalColumnMetrics({
            width, marginLeft, marginRight, gap, columnWidth, columnCount,
            columnGap: vertical ? null : columnGap,
        })
        setStylesImportant(doc.documentElement, {
            'box-sizing': 'border-box',
            'column-width': `${columnWidthHint}px`,
            'column-gap': vertical ? `${(marginTop + marginBottom) * 1.5}px` : `${horizontalColumnGap}px`,
            'column-fill': 'auto',
            ...(vertical
                ? { 'width': `${width}px` }
                : { 'height': `${height}px` }),
            'overflow': 'hidden',
            // force wrap long words
            'overflow-wrap': 'break-word',
            // reset some potentially problematic props
            'position': 'static', 'border': '0', 'margin': '0',
            'max-height': 'none', 'max-width': 'none',
            'min-height': 'none', 'min-width': 'none',
            // fix glyph clipping in WebKit
            '-webkit-line-box-contain': 'block glyphs replaced',
        })
        // Along the pagination axis every page is a tile of `size / columnCount`
        // and the root's own side padding sits inside that tile, so the column's
        // content box is the tile minus the padding. `--available-*` and the
        // image clamp must use the content box: sized to the whole tile, a
        // replaced element runs across the column gap and its trailing edge
        // paints down the margin of the next page (readest/readest#6221).
        // `--full-*` keeps the whole tile, which is what a bleed spans.
        const pageWidth = Math.trunc(width / this.#columnCount)
        const pageHeight = Math.trunc(height / this.#columnCount)
        const availableWidth = vertical
            ? Math.trunc(width - marginLeft / 2 - marginRight / 2 - gap)
            : horizontalAvailableWidth
        const availableHeight = vertical
            ? Math.trunc(height / this.#columnCount - marginTop * 1.5 - marginBottom * 1.5)
            : Math.trunc(height - marginTop - marginBottom)
        setStyles(doc.documentElement, {
            'padding': vertical
                ? `${marginTop * 1.5}px ${marginRight}px ${marginBottom * 1.5}px ${marginLeft}px`
                : `${marginTop}px ${sidePaddingRight}px ${marginBottom}px ${sidePaddingLeft}px`,
            '--page-margin-top': `${vertical ? marginTop * 1.5 : marginTop}px`,
            '--page-margin-right': `${vertical ? marginRight : sidePaddingRight}px`,
            '--page-margin-bottom': `${vertical ? marginBottom * 1.5 : marginBottom}px`,
            '--page-margin-left': `${vertical ? marginLeft : sidePaddingLeft}px`,
            '--full-width': `${vertical ? availableWidth : pageWidth}`,
            '--full-height': `${vertical ? pageHeight : availableHeight}`,
            '--available-width': `${availableWidth}`,
            '--available-height': `${availableHeight}`,
        })
        setStylesImportant(doc.body, {
            'max-height': 'none',
            'max-width': 'none',
            'margin': '0',
            // Prevent position:absolute/fixed on body from coupling its
            // size to the iframe, which causes diverging expand() loops
            'position': 'static',
        })
        this.setImageSize(availableWidth, availableHeight)
        this.#demoteUnfragmentableBoxes(availableHeight)
        this.expand()
    }
    // Atomic inline-level boxes (inline-block / inline-flex / inline-grid /
    // inline-table) cannot be fragmented across columns. When an EPUB declares
    // such a display on a tall block container, the box overflows the page and
    // every column past the first is clipped, so whole sections silently vanish
    // (e.g. a chapter that jumps straight to its references). Detect the
    // vertical overflow this causes in paginated mode and demote the offending
    // boxes to their fragmentable block-level equivalents so the content
    // paginates normally. The querySelectorAll scan only runs when the document
    // actually overflows its column, which is the (rare) bug case.
    #demoteUnfragmentableBoxes(availableHeight) {
        const doc = this.document
        const root = doc?.documentElement
        if (!root || root.scrollHeight <= root.clientHeight + 1) return
        const view = doc.defaultView
        const fragmentable = {
            'inline-block': 'block',
            'inline-flex': 'flex',
            'inline-grid': 'grid',
            'inline-table': 'table',
        }
        for (const el of doc.body.querySelectorAll('*')) {
            const replacement = fragmentable[view.getComputedStyle(el).display]
            if (replacement && el.getBoundingClientRect().height > availableHeight)
                setStylesImportant(el, { display: replacement })
        }
    }
    setImageSize(availableWidth, availableHeight) {
        const { width, height, marginTop, marginRight, marginBottom, marginLeft } = this.#layout
        const vertical = this.#vertical
        const doc = this.document
        const pageFullscreen = doc.documentElement.hasAttribute('data-duokan-page-fullscreen')
        // The fullscreen treatment pins the image with position:absolute and
        // height:100% so it fills the fixed-height page. That only works in
        // paginated (columnized) mode; in scrolled mode the container height is
        // `auto`, so height:100% resolves to 0 and the cover collapses out of
        // sight (#4379). Apply it only when columnized.
        const applyFullscreen = pageFullscreen && this.#column
        // `canvas` belongs with the other replaced elements: a book is free to
        // size one past the page box, and without the clamp it spills over the
        // column rule and paints on top of the next column's text
        // (readest/readest#6041).
        const els = doc.body.querySelectorAll('img, svg, video, canvas')
        // clear previous inline constraints so we read CSS-authored values,
        // not stale pixel values from a previous resize (#3634)
        for (const el of els) {
            el.style.removeProperty('max-width')
            el.style.removeProperty('max-height')
        }
        // Read every element's style before writing any of them. The margins
        // below are resolved values, so reading one flushes layout, and writing
        // the previous element's clamp dirtied it again — interleaving the two
        // costs a full re-fragmentation of the section per element. A Duokan
        // book marks each footnote with an inline <img>, so a volume kept in one
        // XHTML file reaches thousands of them and paged mode froze for minutes
        // (readest/readest#6155). Batched, the whole pass costs one layout.
        const measured = []
        for (const el of els) {
            // preserve max size if they are already set in CSS
            const { maxHeight, maxWidth, marginLeft: elMarginLeft, marginRight: elMarginRight,
                marginTop: elMarginTop, marginBottom: elMarginBottom }
                = doc.defaultView.getComputedStyle(el)
            measured.push({ el, maxHeight, maxWidth,
                elMarginLeft, elMarginRight, elMarginTop, elMarginBottom })
        }
        // `100%` caps the border box, and the element's own margins sit
        // outside it, so an element the book sized at or past the column
        // still hangs its margins over the edge — the IDPF `trees` canvas
        // carries `margin: 1em` and spilled exactly 2em past the column
        // rule (readest/readest#6041). Cap the margin box instead.
        const fill = (a, b) => {
            const margins = (parseFloat(a) || 0) + (parseFloat(b) || 0)
            return margins > 0 ? `calc(100% - ${margins}px)` : '100%'
        }
        for (const m of measured) {
            const { el, elMarginLeft, elMarginRight, elMarginTop, elMarginBottom } = m
            let { maxHeight, maxWidth } = m
            if (parseInt(maxWidth) > availableWidth) {
                maxWidth = `${availableWidth}px`
            }
            if (parseInt(maxHeight) > availableHeight) {
                maxHeight = `${availableHeight}px`
            }
            setStylesImportant(el, {
                'max-height': vertical
                    ? (maxHeight !== 'none' && maxHeight !== '0px' ? maxHeight
                        : fill(elMarginTop, elMarginBottom))
                    : `${height - (applyFullscreen ? 0 : (marginTop + marginBottom))}px`,
                'max-width': vertical
                    ? `${width - (applyFullscreen ? 0 : (marginLeft + marginRight))}px`
                    : (maxWidth !== 'none' && maxWidth !== '0px' ? maxWidth
                        : fill(elMarginLeft, elMarginRight)),
                'object-fit': 'contain',
                'page-break-inside': 'avoid',
                'break-inside': 'avoid',
                'box-sizing': 'border-box',
            })
            if (applyFullscreen) {
                setStylesImportant(doc.documentElement, {
                    position: 'relative',
                })
                setStylesImportant(el, {
                    position: 'absolute',
                    inset: '0',
                    width: '100%',
                    height: '100%',
                    margin: '0',
                    // fit the whole page while keeping the aspect ratio
                    // ('contain' set for all images above); black bars fill
                    // the leftover space like Duokan's native full-page render
                    'background-color': '#000',
                })
                let ancestor = el.parentElement
                while (ancestor && ancestor !== doc.body) {
                    setStylesImportant(ancestor, {
                        width: '100%',
                        height: '100%',
                        margin: '0',
                        padding: '0',
                        // a positioned wrapper (e.g. from duokan-bleed handling)
                        // would become the containing block for the pinned image,
                        // whose height:100% then resolves against the wrapper's
                        // zero height and the cover vanishes (#5263)
                        position: 'static',
                    })
                    ancestor = ancestor.parentElement
                }
            } else if (pageFullscreen) {
                // Scrolled mode for a fullscreen-cover doc: undo any absolute
                // pinning left over from a previous paginated render so the
                // image flows normally, bounded by the max-height set above
                // (#4379). Without this, toggling paginated -> scrolled keeps
                // the stale position:absolute/height:100% and the cover stays
                // collapsed.
                doc.documentElement.style.removeProperty('position')
                for (const prop of ['position', 'inset', 'width', 'height', 'margin', 'background-color']) {
                    el.style.removeProperty(prop)
                }
                let ancestor = el.parentElement
                while (ancestor && ancestor !== doc.body) {
                    for (const prop of ['width', 'height', 'margin', 'padding', 'position']) {
                        ancestor.style.removeProperty(prop)
                    }
                    ancestor = ancestor.parentElement
                }
            }
        }
    }
    get #zoom() {
        // Safari does not zoom the client rects, while Chrome, Edge and Firefox does
        if (/^((?!chrome|android).)*AppleWebKit/i.test(navigator.userAgent) && !window.chrome) {
            return window.getComputedStyle(this.document.body).zoom || 1.0
        }
        return 1.0
    }
    expand() {
        // Font-ready callbacks can run while a preload yields between phases.
        // Its range and page geometry are not initialized until first render.
        if (!this.document?.documentElement || this.document !== this.#loadedDoc) return
        const { documentElement } = this.document
        if (this.#column) {
            const side = this.#vertical ? 'height' : 'width'
            const otherSide = this.#vertical ? 'width' : 'height'
            const contentRect = this.#contentRange.getBoundingClientRect()
            const rootRect = documentElement.getBoundingClientRect()
            // offset caused by column break at the start of the page
            // which seem to be supported only by WebKit and only for horizontal writing
            const contentStart = this.#vertical ? 0
                : this.#rtl ? rootRect.right - contentRect.right : contentRect.left - rootRect.left
            const contentSize = (contentStart + contentRect[side]) * this.#zoom
            // Size content by individual columns, not full spreads.
            // This allows adjacent sections to share a spread when a
            // section doesn't fill all available columns.
            const columnSize = this.#size / this.#columnCount
            const pageCount = Math.ceil(contentSize / columnSize)
            this.#contentPages = pageCount
            const expandedSize = pageCount * columnSize
            this.#element.style.padding = '0'
            this.#iframe.style[side] = `${expandedSize}px`
            this.#element.style[side] = `${expandedSize}px`
            this.#iframe.style[otherSide] = '100%'
            this.#element.style[otherSide] = '100%'
            // One column per "page" — overflow columns extend into adjacent pages
            documentElement.style[side] = `${columnSize}px`
            if (this.#overlayer) {
                this.#overlayer.element.style.margin = '0'
                this.#overlayer.element.style.left = '0'
                this.#overlayer.element.style.top = '0'
                this.#overlayer.element.style[side] = `${expandedSize}px`
                this.#overlayer.redraw()
            }
        } else {
            const side = this.#vertical ? 'width' : 'height'
            const otherSide = this.#vertical ? 'height' : 'width'
            const contentSize = documentElement.getBoundingClientRect()[side]
            let expandedSize = contentSize
            // If the section has a background image, ensure the view is
            // at least as large as the image scaled to fit the cross axis
            if (this.#bgImageSize) {
                const crossSize = this.#element.getBoundingClientRect()[otherSide]
                if (crossSize > 0) {
                    const { width: imgW, height: imgH } = this.#bgImageSize
                    const scaledSize = this.#vertical
                        ? imgW * crossSize / imgH
                        : imgH * crossSize / imgW
                    expandedSize = Math.max(expandedSize, scaledSize)
                }
            }
            this.#element.style.padding = '0'
            this.#iframe.style[side] = `${expandedSize}px`
            this.#element.style[side] = `${expandedSize}px`
            this.#iframe.style[otherSide] = '100%'
            this.#element.style[otherSide] = '100%'
            if (this.#overlayer) {
                this.#overlayer.element.style.margin = '0'
                this.#overlayer.element.style.left = '0'
                this.#overlayer.element.style.top = '0'
                this.#overlayer.element.style[side] = `${expandedSize}px`
                this.#overlayer.redraw()
            }
        }
        this.onExpand()
    }
    set overlayer(overlayer) {
        this.#overlayer = overlayer
        this.#element.append(overlayer.element)
    }
    get overlayer() {
        return this.#overlayer
    }
    #loupeEl = null
    #loupeScaler = null
    #loupeCursor = null
    // Show a magnifier loupe inside the iframe document.
    // winX/winY are in main-window (screen) coordinates.
    showLoupe(winX, winY, { isVertical, color, gap, margin, radius, magnification }) {
        const doc = this.document
        if (!doc) return

        const frameRect = this.#iframe.getBoundingClientRect()
        // Cursor in iframe-viewport coordinates.
        const vpX = winX - frameRect.left
        const vpY = winY - frameRect.top

        // Cursor in document coordinates (accounts for scroll).
        const scrollX = doc.scrollingElement?.scrollLeft ?? 0
        const scrollY = doc.scrollingElement?.scrollTop ?? 0
        const docX = vpX + scrollX
        const docY = vpY + scrollY

        const MAGNIFICATION = magnification
        const MARGIN = margin

        // Capsule dimensions: elongated along the reading direction.
        // For horizontal text the capsule is wider; for vertical it is taller.
        const shortSide = radius * 2
        const longSide  = Math.round(radius * 3.6)
        const loupeW = isVertical ? shortSide : longSide
        const loupeH = isVertical ? longSide  : shortSide
        const halfW = loupeW / 2
        const halfH = loupeH / 2
        const borderRadius = shortSide / 2  // fully rounded ends

        // Position loupe above the cursor (or to the left for vertical text).
        const GAP = gap
        let loupeLeft = isVertical ? vpX - loupeW - GAP : vpX - halfW
        let loupeTop  = isVertical ? vpY - halfH        : vpY - loupeH - GAP
        loupeLeft = Math.max(MARGIN, Math.min(loupeLeft, frameRect.width  - loupeW - MARGIN))
        loupeTop  = Math.max(MARGIN, Math.min(loupeTop,  frameRect.height - loupeH - MARGIN))

        // CSS-transform math: map document point (docX, docY) to loupe centre.
        //   visual_pos = offset + coord × MAGNIFICATION = halfW (or halfH)
        //   ⟹ offset = half − coord × MAGNIFICATION
        const offsetX = halfW - docX * MAGNIFICATION
        const offsetY = halfH - docY * MAGNIFICATION

        // Build loupe DOM structure once; cache it across hide/show cycles so
        // the expensive body clone is not repeated on every drag start.
        if (!this.#loupeEl || !this.#loupeEl.isConnected) {
            this.#loupeEl = doc.createElement('div')

            // Clone the live body once — inside the iframe the epub's CSS
            // variables, @font-face fonts, and styles apply automatically.
            const bodyClone = doc.body.cloneNode(true)

            // Wrap the clone in a div that replicates documentElement's inline
            // styles (column-width, column-gap, padding, height, etc.) so text
            // flows with the same column layout as the original document.
            const htmlWrapper = doc.createElement('div')
            htmlWrapper.style.cssText = doc.documentElement.style.cssText
            // expand() constrains documentElement's page-axis dimension to one
            // page size (width for horizontal, height for vertical).  Override
            // with the full scroll dimension so all columns are rendered.
            if (this.#vertical)
                htmlWrapper.style.height = `${doc.documentElement.scrollHeight}px`
            else
                htmlWrapper.style.width = `${doc.documentElement.scrollWidth}px`
            htmlWrapper.appendChild(bodyClone)

            this.#loupeScaler = doc.createElement('div')
            this.#loupeScaler.appendChild(htmlWrapper)

            const cursorLen = Math.round(shortSide * 0.44)
            this.#loupeCursor = doc.createElement('div')
            this.#loupeCursor.style.cssText = isVertical
                ? `position:absolute;left:calc(50% - ${cursorLen / 2}px);top:50%;`
                + `margin-top:-1px;width:${cursorLen}px;height:2px;background:${color};pointer-events:none;z-index:1;box-sizing:border-box;`
                : `position:absolute;left:50%;top:calc(50% - ${cursorLen / 2}px);`
                + `margin-left:-1px;width:2px;height:${cursorLen}px;background:${color};pointer-events:none;z-index:1;box-sizing:border-box;`

            this.#loupeEl.appendChild(this.#loupeScaler)
            this.#loupeEl.appendChild(this.#loupeCursor)
            doc.documentElement.appendChild(this.#loupeEl)

            // Static loupe shell styles (set once).
            this.#loupeEl.style.cssText = `
                position: absolute;
                width: ${loupeW}px;
                height: ${loupeH}px;
                border-radius: ${borderRadius}px;
                overflow: hidden;
                border: 2.5px solid ${color};
                box-shadow: 0 6px 24px rgba(0,0,0,0.28);
                background-color: var(--theme-bg-color);
                z-index: 9999;
                pointer-events: none;
                user-select: none;
                box-sizing: border-box;
                contain: strict;
            `

            // Static scaler styles (set once; only left/top change per move).
            this.#loupeScaler.style.cssText = `
                position: absolute;
                transform: scale(${MAGNIFICATION});
                transform-origin: 0 0;
                pointer-events: none;
            `
        }

        // Ensure visible (hideLoupe hides via CSS instead of removing).
        this.#loupeEl.style.display = ''

        // Update only the dynamic position values (fast path on every move).
        this.#loupeScaler.style.left = `${offsetX}px`
        this.#loupeScaler.style.top = `${offsetY}px`
        this.#loupeScaler.style.width = `${doc.documentElement.scrollWidth}px`
        this.#loupeScaler.style.height = `${doc.documentElement.scrollHeight}px`
        this.#loupeEl.style.left = `${loupeLeft + scrollX}px`
        this.#loupeEl.style.top = `${loupeTop + scrollY}px`

        // Cut a capsule-shaped hole in the overlayer so highlights don't paint
        // over the loupe.
        if (this.#overlayer) {
            const overlayerRect = this.#overlayer.element.getBoundingClientRect()
            const dx = frameRect.left - overlayerRect.left
            const dy = frameRect.top - overlayerRect.top

            const pad = 3
            const cx = loupeLeft + halfW + dx
            const cy = loupeTop + halfH + dy

            this.#overlayer.setHole(cx, cy, loupeW + pad * 2, loupeH + pad * 2, borderRadius + pad)
        }
    }
    hideLoupe() {
        // Hide via CSS instead of removing — keeps the cached body clone so
        // the next showLoupe call skips the expensive cloneNode(true).
        if (this.#loupeEl) {
            this.#loupeEl.style.display = 'none'
        }
        if (this.#overlayer)
            this.#overlayer.clearHole()
    }
    destroyLoupe() {
        if (this.#loupeEl) {
            this.#loupeEl.remove()
            this.#loupeEl = null
            this.#loupeScaler = null
            this.#loupeCursor = null
        }
        if (this.#overlayer)
            this.#overlayer.clearHole()
    }
    destroy() {
        if (this.document?.body) this.#observer.unobserve(this.document.body)
        this.destroyLoupe()
    }
}

// Horizontal metrics of a paginated column layout: the CSS column gap, the
// root's side padding, the `column-width` hint and the content width of one
// column. The document is one multi-column flow that the container scrolls
// through a page at a time, so a page tiles only when the side paddings add
// up to the column gap: by default each side takes a quarter of its margin
// plus a quarter of the gap and the centre gets the sum, which makes every
// page edge look the same. A `column-gap` override sets the centre gap and
// splits it evenly over the two sides; the outer edge of the text is kept by
// the host grid instead (getColumnGapHostTracks). The width hint shrinks with
// the gap: the browser fits floor((content + gap) / (hint + gap)) columns, so
// the derived hint under a wider gap collapses the spread to a single column.
export const getHorizontalColumnMetrics = ({ width, marginLeft, marginRight, gap, columnWidth, columnCount, columnGap }) => {
    const multi = columnCount > 1
    if (!multi || !Number.isFinite(columnGap) || columnGap < 0) {
        const sidePaddingLeft = multi ? marginLeft / 4 + gap / 4 : marginLeft / 2 + gap / 2
        const sidePaddingRight = multi ? marginRight / 4 + gap / 4 : marginRight / 2 + gap / 2
        return {
            columnGap: multi ? (marginLeft + marginRight) / 4 + gap / 2 : (marginLeft + marginRight) / 2 + gap,
            sidePaddingLeft,
            sidePaddingRight,
            columnWidth: Math.trunc(columnWidth),
            availableWidth: Math.trunc(width / columnCount - sidePaddingLeft - sidePaddingRight),
        }
    }
    const sidePadding = columnGap / 2
    const column = Math.trunc(width / columnCount - columnGap)
    return { columnGap, sidePaddingLeft: sidePadding, sidePaddingRight: sidePadding, columnWidth: column, availableWidth: column }
}

// How the host grid keeps the text's outer edge under a `column-gap`
// override. #container spans the margin tracks, so the only host space
// outside a page is the flexible outer track, whose minimum matches the
// page's derived side padding: by default the outer edge of the text is
// about the derived gap wide (outer minimum + side padding), the same as the
// centre. Under an override the side padding becomes half the gap, so the
// outer minimum gives up the difference and the container cap grows by the
// whole of it: a capped spread widens rather than squeezing its columns, and
// an uncapped one takes the room from the outer tracks. The minimum cannot
// go below zero, so past twice the derived gap the outer edge moves inward
// with half the gap: on a page the gutter is at most about twice the outer
// edge. `hostGap` is the gap percentage resolved against the host, which is
// what the grid's own tracks use; the page's padding resolves it against
// the container instead, so the edge can drift by a pixel or two.
export const getColumnGapHostTracks = ({ columnGap, marginLeft, marginRight, hostGap }) => {
    const derivedLeft = marginLeft / 4 + hostGap / 4
    const derivedRight = marginRight / 4 + hostGap / 4
    return {
        outerMinLeft: Math.max(0, 2 * derivedLeft - columnGap / 2),
        outerMinRight: Math.max(0, 2 * derivedRight - columnGap / 2),
        capExtra: columnGap - derivedLeft - derivedRight,
    }
}

// NOTE: everything here assumes the so-called "negative scroll type" for RTL
export class Paginator extends HTMLElement {
    static observedAttributes = [
        'flow', 'gap', 'column-gap', 'margin-top', 'margin-bottom', 'margin-left', 'margin-right',
        'max-inline-size', 'max-block-size', 'max-column-count',
        'no-preload', 'no-background', 'no-continuous-scroll',
    ]
    #root = this.attachShadow({ mode: 'open' })
    #observer = new ResizeObserver(() => this.render())
    #top
    #background
    #container
    #header
    #footer
    #views = new Map() // Map<sectionIndex, View>
    #primaryIndex = -1
    #vertical = false
    #rtl = false
    #marginTop = 0
    #marginBottom = 0
    #anchor = 0 // anchor view to a fraction (0-1), Range, or Element
    #justAnchored = false
    #locked = false // while true, prevent any further navigation
    #styles
    #styleMap = new WeakMap()
    #mediaQuery = matchMedia('(prefers-color-scheme: dark)')
    #mediaQueryListener
    #scrollBounds
    #touchState
    #touchScrolled
    #lastVisibleRange
    #scrollLocked = false
    #subpixelOffset = 0
    #isAnimating = false
    // Generation counter for slideTurnAnimation: a newer vertical page turn
    // bumps it so an in-flight two-phase slide stops touching the DOM.
    #slideTurnId = 0
    // Horizontal drag offset (px) applied to the views while a finger tracks
    // a page turn on a vertical book; consumed as the slide's start position
    // when the turn commits, or settled back to 0 when it doesn't.
    #dragTranslateX = 0
    // Active finger-tracked layered turn (readest#555): a paused view
    // transition whose animations are scrubbed by the drag.
    #vtDrag = null
    // A released layered turn still owns the global View Transition until its
    // commit/cancel cleanup and terminal lifecycle event complete.
    #vtFinishing = null
    #vtProgrammatic = null
    #vtNamedHost = null
    // Snapshot of the invariant inputs #replaceBackground needs (theme/texture
    // style, background+container geometry, per-view size+colour). Set once when
    // a scroll animation starts so the per-frame repaint reuses it instead of
    // forcing a fresh style+layout read every frame; null when not animating.
    #bgAnimContext = null
    #filling = false // true while #fillVisibleArea is running
    #fillPromise = null // tracks in-progress #fillVisibleArea for awaiting
    #stabilizing = false // true while #display is stabilizing layout
    #rendered = false // true after first #display completes
    #lastLayout = null // cached layout from the last #beforeRender call
    #columnGap = null // px; set by `column-gap`, overrides the centre gap of a spread
    // Cache of section index → vertical (boolean). Populated as views
    // are loaded so we can check direction *before* loading a section.
    #directionCache = new Map()
    constructor() {
        super()
        this.#root.innerHTML = `<style>
        :host {
            display: block;
            container-type: size;
        }
        :host, #top {
            box-sizing: border-box;
            position: relative;
            overflow: hidden;
            width: 100%;
            height: 100%;
        }
        #top {
            --_gap: 7%;
            --_margin-top: 48px;
            --_margin-right: 48px;
            --_margin-bottom: 48px;
            --_margin-left: 48px;
            --_max-inline-size: 720px;
            --_max-block-size: 1440px;
            --_max-column-count: 2;
            --_max-column-count-portrait: var(--_max-column-count);
            --_max-column-count-spread: var(--_max-column-count);
            --_half-gap: calc(var(--_gap) / 2);
            --_half-margin-left: calc(var(--_margin-left) / 2);
            --_half-margin-right: calc(var(--_margin-right) / 2);
            --_max-width: calc(var(--_max-inline-size) * var(--_max-column-count-spread));
            --_max-height: var(--_max-block-size);
            --_column-count: 1;
            --_outer-min-left: calc((var(--_column-count) - 1) * (var(--_margin-left) / 4 + var(--_gap) / 4));
            --_outer-min-right: calc((var(--_column-count) - 1) * (var(--_margin-right) / 4 + var(--_gap) / 4));
            /* A column-gap override lowers the outer minimums and widens the
               cap so the text keeps its outer edge; see getColumnGapHostTracks. */
            --_gap-cap-extra: 0px;
            display: grid;
            grid-template-columns:
                minmax(var(--_outer-min-left), 1fr)
                var(--_margin-left)
                minmax(0, calc(var(--_max-width) - var(--_gap) + var(--_gap-cap-extra)))
                var(--_margin-right)
                minmax(var(--_outer-min-right), 1fr);
            grid-template-rows:
                minmax(var(--_margin-top), 1fr)
                minmax(0, var(--_max-height))
                minmax(var(--_margin-bottom), 1fr);
            &.vertical {
                --_max-column-count-spread: var(--_max-column-count-portrait);
                --_max-width: var(--_max-block-size);
                --_max-height: calc(var(--_max-inline-size) * var(--_max-column-count-spread));
            }
            @container (orientation: portrait) {
                & {
                    --_max-column-count-spread: var(--_max-column-count-portrait);
                }
                &.vertical {
                    --_max-column-count-spread: var(--_max-column-count);
                }
            }
        }
        #background {
            grid-column: 1 / -1;
            grid-row: 1 / -1;
            position: relative;
            overflow: hidden;
        }
        #container {
            grid-column: 2 / 5;
            grid-row: 1 / -1;
            overflow: hidden;
            display: flex;
            flex-direction: row;
            transition: opacity 50ms ease-in;
        }
        #container.vertical {
            flex-direction: column;
        }
        /* Apple WebKit (iOS/macOS) composites large, persistent layers without
           the ~1s Blink freeze Android Chromium hits at high DPR (the reason
           these promotion hints were dropped and rafAnimateScroll was added).
           When the host opts in, restore persistent compositor layers for the
           container and each view so the GPU cssAnimateScroll page-turn stays
           smooth on 120Hz ProMotion instead of promoting a layer on-demand
           every turn (readest#4768). Paginated mode only; scrolled mode does
           its own compositing below. */
        :host([gpu-composite]:not([flow="scrolled"])) #container,
        :host([gpu-composite]:not([flow="scrolled"])) #container > * {
            transform: translateZ(0);
        }
        :host([flow="scrolled"]) #container {
            grid-column: 2 / 5;
            grid-row: 1 / -1;
            overflow: auto;
            overflow-anchor: auto;
            flex-direction: column;
            /* Composite the scroll container so its scrollbar repaints on the
               compositor thread; the main-thread scrollbar fails to
               re-invalidate after content-size changes (adjacent-section
               preloading right after open), so on Windows' always-on
               scrollbars it vanishes shortly after the book opens
               (readest#4470). Scoped to scrolled mode to leave the paginated
               page-turn path un-composited. */
            transform: translateZ(0);
        }
        :host([flow="scrolled"]) #container.vertical {
            flex-direction: row;
        }
        #header {
            grid-column: 3 / 4;
            grid-row: 1;
        }
        #footer {
            grid-column: 3 / 4;
            grid-row: 3;
            align-self: end;
        }
        #header {
            display: grid;
            height: var(--_margin-top);
        }
        #footer {
            display: grid;
            height: var(--_margin-bottom);
        }
        :is(#header, #footer) > * {
            display: flex;
            align-items: center;
            min-width: 0;
        }
        :is(#header, #footer) > * > * {
            width: 100%;
            overflow: hidden;
            white-space: nowrap;
            text-overflow: ellipsis;
            text-align: center;
            font-size: .75em;
            opacity: .6;
        }
        </style>
        <div id="top">
            <div id="background" part="filter"></div>
            <div id="header"></div>
            <div id="container" part="container"></div>
            <div id="footer"></div>
        </div>
        `

        this.#top = this.#root.getElementById('top')
        this.#background = this.#root.getElementById('background')
        this.#container = this.#root.getElementById('container')
        this.#header = this.#root.getElementById('header')
        this.#footer = this.#root.getElementById('footer')

        this.#observer.observe(this.#container)
        const scrolledScrollRelocate = () => {
            // Skip entirely while stabilizing — preserve #justAnchored
            // so the first post-stabilization fire still sees it.
            if (this.#stabilizing) return
            if (this.#justAnchored) this.#justAnchored = false
            else this.#afterScroll('scroll')
        }
        // Start time of the current unbroken run of scroll events, for the
        // periodic mid-scroll relocate below; null when no run is in progress.
        let scrollBurstStart = null
        const debouncedScroll = debounce(() => {
            scrollBurstStart = null
            if (this.scrolled && !this.#isAnimating) {
                scrolledScrollRelocate()
                // Backward preloading is handled eagerly in the (non-debounced)
                // scroll listener below, mirroring the forward buffer.
            } else if (!this.scrolled) {
              this.#afterScroll('container-scroll')
            }
        }, 250)
        this.#container.addEventListener('scroll', () => {
            if (!this.#isAnimating) this.dispatchEvent(new Event('scroll'))
            // Keep the per-view backgrounds glued to the content while a swipe
            // drag scrolls the container (no animation runs then). During the
            // snap animation #isAnimating is set and the destination background
            // is already in place, so the rebuild is skipped.
            if (!this.scrolled && !this.#isAnimating) this.#replaceBackground()
            // Preload forward when fewer than minPages ahead. Skip while a finger
            // drag is in progress: loading a section runs columnize/expand on the
            // main thread, which drops frames mid-swipe (readest#4785). The buffer
            // is still 4+ pages deep during a one-page drag, and the scroll that
            // settles the gesture re-fires this with the finger already up, so the
            // top-up just moves off the active drag instead of being skipped.
            if (!this.noPreload && !this.noContinuousScroll && !this.#filling
                && !this.#stabilizing && !this.#touchScrolled) {
                const minPages = 5
                const pagesAhead = this.size > 0
                    ? Math.floor((this.#renderedViewSize - this.#renderedEnd) / this.size)
                    : 0
                if (pagesAhead < minPages) {
                    const sorted = this.#sortedViews
                    const lastIndex = sorted[sorted.length - 1]?.[0]
                    if (lastIndex != null) {
                        const nextIdx = this.#adjacentIndex(1, lastIndex)
                        if (nextIdx != null && !this.#views.has(nextIdx) && this.#isSameDirection(nextIdx)) {
                            this.#filling = true
                            this.#loadAdjacentSection(nextIdx)
                                .finally(() => {
                                    this.#filling = false
                                    this.dispatchEvent(new Event('stabilized'))
                                })
                        }
                    }
                }
            }
            // Preload backward when fewer than minPages behind, mirroring the
            // forward buffer so scrolling up never dead-ends at the top with the
            // previous section unloaded (readest/readest#4112). The
            // #loadAdjacentSection scroll compensation keeps the viewport
            // anchored as the section is inserted above.
            if (this.scrolled && !this.noPreload && !this.noContinuousScroll
                && !this.#filling && !this.#stabilizing) {
                const minPages = 5
                const pagesBehind = this.size > 0
                    ? Math.floor(this.#renderedStart / this.size)
                    : 0
                if (pagesBehind < minPages) {
                    const sorted = this.#sortedViews
                    const firstIndex = sorted[0]?.[0]
                    if (firstIndex != null) {
                        const prevIdx = this.#adjacentIndex(-1, firstIndex)
                        if (prevIdx != null && !this.#views.has(prevIdx) && this.#isSameDirection(prevIdx)) {
                            this.#filling = true
                            this.#loadAdjacentSection(prevIdx)
                                .finally(() => {
                                    this.#filling = false
                                    this.dispatchEvent(new Event('stabilized'))
                                })
                        }
                    }
                }
            }
            // A scroll that never pauses — the Auto Scroll reading mode, a
            // held scroll key — resets the trailing debounce forever, so the
            // scrolled-mode relocate (and with it reading progress) would only
            // fire once the scrolling stopped (readest#5635). Relocate at most
            // once per SCROLL_RELOCATE_MAX_WAIT while the run of scroll events
            // lasts; the debounced call still reports the final position and
            // ends the run. Skipped during a finger drag for the same reason
            // preloading is: the measurement drops frames mid-swipe, and the
            // release settles through the debounce anyway (readest#4785).
            if (this.scrolled && !this.#isAnimating && !this.#touchScrolled) {
                const now = Date.now()
                if (scrollBurstStart == null) scrollBurstStart = now
                else if (now - scrollBurstStart >= SCROLL_RELOCATE_MAX_WAIT) {
                    scrollBurstStart = now
                    scrolledScrollRelocate()
                }
            }
            debouncedScroll()
        })

        const opts = { passive: false }
        this.addEventListener('touchstart', this.#onTouchStart.bind(this), opts)
        this.addEventListener('touchmove', this.#onTouchMove.bind(this), opts)
        this.addEventListener('touchend', this.#onTouchEnd.bind(this))
        this.addEventListener('touchcancel', this.#onTouchCancel.bind(this))
        this.addEventListener('load', ({ detail: { doc } }) => {
            doc.addEventListener('touchstart', this.#onTouchStart.bind(this), opts)
            doc.addEventListener('touchmove', this.#onTouchMove.bind(this), opts)
            doc.addEventListener('touchend', this.#onTouchEnd.bind(this))
            doc.addEventListener('touchcancel', this.#onTouchCancel.bind(this))
        })

        this.addEventListener('relocate', ({ detail }) => {
            if (detail.reason === 'selection') setSelectionTo(this.#anchor, 0)
            else if (detail.reason === 'navigation') {
                if (this.#anchor === 1) setSelectionTo(detail.range, 1)
                else if (typeof this.#anchor === 'number')
                    setSelectionTo(detail.range, -1)
                else setSelectionTo(this.#anchor, -1)
            }
        })
        const checkPointerSelection = debounce((range, sel) => {
            if (!sel.rangeCount) return
            const selRange = sel.getRangeAt(0)
            const backward = selectionIsBackward(sel)
            if (backward && selRange.compareBoundaryPoints(Range.START_TO_START, range) < 0)
                this.prev()
            else if (!backward && selRange.compareBoundaryPoints(Range.END_TO_END, range) > 0)
                this.next()
        }, 700)
        this.addEventListener('load', ({ detail: { doc } }) => {
            let isPointerSelecting = false
            doc.addEventListener('pointerdown', () => isPointerSelecting = true)
            doc.addEventListener('pointerup', () => isPointerSelecting = false)
            let isKeyboardSelecting = false
            doc.addEventListener('keydown', () => isKeyboardSelecting = true)
            doc.addEventListener('keyup', () => isKeyboardSelecting = false)
            doc.addEventListener('selectionchange', () => {
                if (this.scrolled) return
                const range = this.#lastVisibleRange
                if (!range) return
                const sel = doc.getSelection()
                if (!sel.rangeCount) return
                // FIXME: this won't work on Android WebView, disable for now
                if (!isPointerSelecting && isPointerSelecting && sel.type === 'Range')
                    checkPointerSelection(range, sel)
                else if (isKeyboardSelecting) {
                    const selRange = sel.getRangeAt(0).cloneRange()
                    const backward = selectionIsBackward(sel)
                    if (!backward) selRange.collapse()
                    this.#scrollToAnchor(selRange)
                }
            })
            doc.addEventListener('focusin', e => {
                if (this.scrolled) return null
                if (this.#container && this.#container.contains(e.target)) {
                    // NOTE: `requestAnimationFrame` is needed in WebKit
                    requestAnimationFrame(() => this.#scrollToAnchor(e.target))
                }
            })
        })

        this.#mediaQueryListener = () => {
            const view = this.#primaryView
            if (!view) return
            this.#replaceBackground()
        }
        this.#mediaQuery.addEventListener('change', this.#mediaQueryListener)
    }
    get #primaryView() {
        return this.#views.get(this.#primaryIndex)
    }
    get #sortedViews() {
        return [...this.#views.entries()].sort(([a], [b]) => a - b)
    }
    get primaryIndex() {
        return this.#primaryIndex
    }
    setAttribute(name, value) {
        // The scrolled-mode scroll handler is debounced, so #anchor and
        // #primaryIndex can lag behind the user's actual viewport by up to
        // ~250ms. Toggling out of scrolled mode within that window made
        // render() restore the stale anchor — reverting the position to a
        // previously visible section. Flush the pending scroll state here,
        // before the attribute change so the layout is still in scrolled
        // mode and `this.scrolled` (which reads the attribute) is still true.
        if (name === 'flow'
            && this.scrolled
            && String(value) !== 'scrolled'
            && this.#views.size > 0) {
            this.#flushScrolledState()
        }
        super.setAttribute(name, value)
    }
    #flushScrolledState() {
        if (this.#views.size > 1) this.#detectPrimaryView()
        const result = this.#getVisibleRange()
        if (result?.range && !result.range.collapsed) this.#anchor = result.range
    }
    attributeChangedCallback(name, _, value) {
        switch (name) {
            case 'flow':
                this.render()
                break
            case 'column-gap': {
                const px = parseFloat(value)
                this.#columnGap = Number.isFinite(px) ? px : null
                this.render()
                break
            }
            case 'gap':
            case 'margin-top':
            case 'margin-bottom':
            case 'margin-left':
            case 'margin-right':
            case 'max-block-size':
            case 'max-column-count':
                this.#top.style.setProperty('--_' + name, value)
                this.render()
                break
            case 'max-inline-size':
                // needs explicit `render()` as it doesn't necessarily resize
                this.#top.style.setProperty('--_' + name, value)
                this.render()
                break
            case 'no-continuous-scroll':
                if (this.noContinuousScroll) {
                    for (const [i] of this.#views) {
                        if (i !== this.#primaryIndex) this.#destroyView(i)
                    }
                }
                break
        }
    }
    open(book) {
        this.bookDir = book.dir
        this.sections = book.sections
        book.transformTarget?.addEventListener('data', ({ detail }) => {
            if (detail.type !== 'text/css') return
            detail.data = Promise.resolve(detail.data).then(data => data
                // unprefix as most of the props are (only) supported unprefixed
                .replace(/([{\s;])-epub-/gi, '$1')
                // `page-break-*` unsupported in columns; replace with `column-break-*`
                .replace(/page-break-(after|before|inside)\s*:/gi, (_, x) =>
                    `-webkit-column-break-${x}:`)
                .replace(/break-(after|before|inside)\s*:\s*(avoid-)?page/gi, (_, x, y) =>
                    `break-${x}: ${y ?? ''}column`))
        })
    }
    #createView(index) {
        // Destroy existing view for this index if any
        const existing = this.#views.get(index)
        if (existing) {
            existing.destroy()
            this.#container.removeChild(existing.element)
            this.#views.delete(index)
        }
        const view = new View({
            container: this,
            onExpand: () => {
                // Only the primary view's resize should adjust scroll;
                // non-primary views (preloaded/adjacent) must not scroll
                if (this.#filling || this.#stabilizing || this.scrolled) return
                if (this.#primaryIndex === index)
                    this.#scrollToAnchor(this.#anchor)
            },
        })
        this.#views.set(index, view)
        const sorted = this.#sortedViews
        const myPos = sorted.findIndex(([i]) => i === index)
        const nextEntry = sorted[myPos + 1]
        if (nextEntry) this.#container.insertBefore(view.element, nextEntry[1].element)
        else this.#container.append(view.element)
        this.#syncA11y()
        return view
    }
    // Hide off-screen pre-loaded views from the accessibility tree so
    // screen-reader swipe-next does not wander into them (which would land
    // several pages into the next section instead of its first paragraph).
    //
    // Only `aria-hidden` is used — `inert` would also block pointer events
    // and text selection, which breaks visible non-primary views such as
    // the right column of a dual-page spread when each column belongs to
    // a different section (readest/readest#4243, readest/readest#4259).
    //
    // Visible non-primary views stay exposed to assistive tech because a
    // sighted user can read them on the same spread.
    #syncA11y() {
        const containerRect = this.#container.getBoundingClientRect()
        for (const [index, view] of this.#views) {
            const isPrimary = index === this.#primaryIndex
            const isVisible = isPrimary
                || isViewVisibleInContainer(
                    view.element.getBoundingClientRect(), containerRect)
            if (isVisible) view.element.removeAttribute('aria-hidden')
            else view.element.setAttribute('aria-hidden', 'true')
        }
    }
    #destroyView(index) {
        const view = this.#views.get(index)
        if (!view) return
        view.destroy()
        this.#container.removeChild(view.element)
        this.#views.delete(index)
        this.sections[index]?.unload?.()
    }
    #destroyAllViews() {
        for (const [index] of this.#views) this.#destroyView(index)
    }
    #clearViewsExcept(keepIndices) {
        for (const [index] of this.#views) {
            if (!keepIndices.has(index)) this.#destroyView(index)
        }
    }
    // Check if a section has the same writing direction as current primary.
    // Returns true if same or unknown (not yet cached).
    #isSameDirection(index) {
        if (!this.#directionCache.has(index)) return true
        return this.#directionCache.get(index) === this.#vertical
    }
    // Read the theme/texture style off the primary section's <html> and return a
    // resolver that maps a view's raw background onto the active theme. This is a
    // forced style read (getComputedStyle), so callers in the animation hot path
    // do it once via #computePaginatedBgContext rather than every frame.
    #readBackgroundStyle(doc) {
        const htmlStyle = doc.defaultView.getComputedStyle(doc.documentElement)
        const themeBgColor = htmlStyle.getPropertyValue('--theme-bg-color')
        const overrideColor = htmlStyle.getPropertyValue('--override-color') === 'true'
        const bgTextureId = htmlStyle.getPropertyValue('--bg-texture-id')
        const isDarkMode = htmlStyle.getPropertyValue('color-scheme') === 'dark'
        const fallbackBg = themeBgColor || ''
        const hasTexture = !!bgTextureId && bgTextureId !== 'none'

        const resolveBackground = (background) => {
            if (!background) return fallbackBg
            if (themeBgColor) {
                const parsed = background.split(/\s(?=(?:url|rgb|hsl|#[0-9a-fA-F]{3,6}))/)
                if ((isDarkMode || overrideColor) && (bgTextureId === 'none' || !bgTextureId)) {
                    parsed[0] = themeBgColor
                }
                return parsed.join(' ')
            }
            return background
        }
        return { fallbackBg, hasTexture, resolveBackground }
    }
    // Snapshot every input #paintPaginatedBackground needs that stays constant for
    // the duration of a scroll animation: the theme/texture style, the
    // background+container geometry, and each rendered view's size and resolved
    // background. Returns null when there is nothing to paint (no primary doc,
    // backgrounds disabled, or scrolled mode). Built once per animation so the
    // per-frame repaint never re-runs getComputedStyle or one
    // getBoundingClientRect per view — those forced reads, multiplied by the views
    // preloaded at a chapter boundary, are what dropped frames mid-swipe
    // (readest#4785).
    #computePaginatedBgContext() {
        const doc = this.#primaryView?.document
        if (!doc?.documentElement) return null
        if (this.noBackground) return null
        if (this.scrolled) return null
        const { fallbackBg, hasTexture, resolveBackground } = this.#readBackgroundStyle(doc)
        const bgRect = this.#background.getBoundingClientRect()
        const containerRect = this.#container.getBoundingClientRect()
        const startEdge = this.#vertical ? 'top' : 'left'
        const bgSize = bgRect[this.sideProp]
        const inset = containerRect[startEdge] - bgRect[startEdge]
        const containerSize = containerRect[this.sideProp]
        const views = this.#sortedViews.map(([, view]) => ({
            size: view.element.getBoundingClientRect()[this.sideProp],
            bg: textureAwareBackground(resolveBackground(view.docBackground), hasTexture),
        }))
        return { fallbackBg, hasTexture, bgSize, inset, containerSize, views }
    }
    // Paint one full-bleed background segment per rendered view from a previously
    // computed context, positioned so each tracks its content on screen at the
    // given scroll position. Rebuilding on every scroll keeps the backgrounds
    // glued to the content during a swipe drag — so when two sections with
    // different backgrounds are both visible, each half shows its own colour
    // instead of one flat colour flashing across the viewport. Only writes layout
    // (no reads), so it is safe to run every animation frame.
    #paintPaginatedBackground(ctx, atPosition) {
        // Reset any inline backgrounds left over from a previous mode so the
        // host's texture isn't occluded after toggling.
        this.#background.style.background = ''
        for (const [, view] of this.#sortedViews) {
            view.element.style.background = ''
        }
        const scrollPos = Math.abs(atPosition ?? this.#renderedStart)
        const segments = computeBackgroundSegments(
            ctx.views, scrollPos, ctx.bgSize, ctx.inset, ctx.containerSize)

        this.#background.innerHTML = ''
        this.#background.style.display = ''
        // Under a texture, leave the container transparent so the host texture
        // shows through the gaps a transparent page no longer fills (readest#4399).
        this.#background.style.background = ctx.hasTexture ? '' : ctx.fallbackBg

        const posProp = this.#vertical ? 'top' : 'left'
        const sizeProp = this.#vertical ? 'height' : 'width'
        const crossPosProp = this.#vertical ? 'left' : 'top'
        const crossSizeProp = this.#vertical ? 'width' : 'height'
        for (const { start, size, bg } of segments) {
            const seg = document.createElement('div')
            seg.style.position = 'absolute'
            seg.style[posProp] = `${start}px`
            seg.style[sizeProp] = `${size}px`
            seg.style[crossPosProp] = '0'
            seg.style[crossSizeProp] = '100%'
            seg.style.background = bg
            seg.style.backgroundAttachment = 'initial'
            this.#background.appendChild(seg)
        }
    }
    // Update the #background grid so each column shows the correct section's
    // background. Pass atPosition to pre-compute for a destination scroll
    // position (e.g. before an animation starts). During a scroll animation the
    // invariant context is snapshotted in #bgAnimContext and reused here so the
    // per-frame repaint stays read-free.
    #replaceBackground(atPosition) {
        const doc = this.#primaryView?.document
        if (!doc?.documentElement) return
        if (this.noBackground) return

        if (this.scrolled) {
            // In scrolled mode, set background directly on each view element
            // so it scrolls with the content. The static #background provides
            // the fallback color for margins and gaps between views.
            const { fallbackBg, hasTexture, resolveBackground } = this.#readBackgroundStyle(doc)
            this.#background.style.background = ''
            this.#background.innerHTML = ''
            this.#background.style.display = ''
            this.#background.style.background = hasTexture ? '' : fallbackBg
            for (const [, view] of this.#sortedViews) {
                const resolved = resolveBackground(view.docBackground)
                view.element.style.background = textureAwareBackground(resolved, hasTexture)
            }
            return
        }

        const ctx = this.#bgAnimContext ?? this.#computePaginatedBgContext()
        if (!ctx) return
        this.#paintPaginatedBackground(ctx, atPosition)
    }
    #beforeRender({ vertical, rtl: documentRTL }) {
        // If writing-mode is about to change, destroy all non-primary
        // views BEFORE updating global state. This prevents stale views
        // with the wrong direction from remaining in the container while
        // flex-direction / scrollProp / sideProp flip.
        if (this.#rendered && vertical !== this.#vertical) {
            for (const [i] of this.#views) {
                if (i !== this.#primaryIndex) this.#destroyView(i)
            }
        }
        this.#vertical = vertical
        // One scroll container holds the views of several sections at once, so
        // the progression cannot be per-document: a section that flipped it
        // would reverse the sections mounted beside it. The publication-wide
        // declaration settles it for the whole book; only a book that declares
        // nothing still follows its documents.
        this.#rtl = getPageProgressionRTL(this.bookDir, documentRTL)
        this.#top.classList.toggle('vertical', vertical)
        this.#container.classList.toggle('vertical', vertical)

        const style = getComputedStyle(this.#top)
        const maxInlineSize = parseFloat(style.getPropertyValue('--_max-inline-size'))
        const maxColumnCount = parseInt(style.getPropertyValue('--_max-column-count-spread'))
        const marginTop = parseFloat(style.getPropertyValue('--_margin-top'))
        const marginRight = parseFloat(style.getPropertyValue('--_margin-right'))
        const marginBottom = parseFloat(style.getPropertyValue('--_margin-bottom'))
        const marginLeft = parseFloat(style.getPropertyValue('--_margin-left'))
        this.#marginTop = marginTop
        this.#marginBottom = marginBottom

        // Compute the column count from the host (Paginator) size rather than
        // the #container size. The container width depends on --_column-count
        // via the grid template (the outer 1fr tracks have a non-zero min for
        // multi-column spreads), so deriving the column count from container
        // size at threshold widths creates a feedback loop where the layout
        // oscillates between 1 and 2 columns on resize.
        const flow = this.getAttribute('flow')
        const hostRect = this.getBoundingClientRect()
        const hostSize = vertical ? hostRect.height : hostRect.width
        const divisor = flow === 'scrolled'
            ? 1
            : Math.min(
                maxColumnCount + (vertical ? 1 : 0),
                Math.ceil(Math.floor(hostSize) / Math.floor(maxInlineSize)),
            )
        // Set --_column-count BEFORE measuring the container so the read
        // below reflects the grid template that will actually be used.
        this.#top.style.setProperty('--_column-count', divisor)
        // Same for the `column-gap` override's tracks (see getColumnGapHostTracks).
        const columnGap = flow !== 'scrolled' && !vertical && divisor > 1 ? this.#columnGap : null
        if (columnGap != null) {
            const hostGap = parseFloat(style.getPropertyValue('--_gap')) / 100 * hostSize
            const tracks = getColumnGapHostTracks({ columnGap, marginLeft, marginRight, hostGap })
            this.#top.style.setProperty('--_outer-min-left', `${tracks.outerMinLeft}px`)
            this.#top.style.setProperty('--_outer-min-right', `${tracks.outerMinRight}px`)
            this.#top.style.setProperty('--_gap-cap-extra', `${tracks.capExtra}px`)
        } else {
            this.#top.style.removeProperty('--_outer-min-left')
            this.#top.style.removeProperty('--_outer-min-right')
            this.#top.style.setProperty('--_gap-cap-extra', '0px')
        }

        const { width, height } = this.#container.getBoundingClientRect()
        const size = vertical ? height : width

        const g = parseFloat(style.getPropertyValue('--_gap')) / 100
        // The gap will be a percentage of the #container, not the whole view.
        // This means the outer padding will be bigger than the column gap. Let
        // `a` be the gap percentage. The actual percentage for the column gap
        // will be (1 - a) * a. Let us call this `b`.
        //
        // To make them the same, we start by shrinking the outer padding
        // setting to `b`, but keep the column gap setting the same at `a`. Then
        // the actual size for the column gap will be (1 - b) * a. Repeating the
        // process again and again, we get the sequence
        //     x₁ = (1 - b) * a
        //     x₂ = (1 - x₁) * a
        //     ...
        // which converges to x = (1 - x) * a. Solving for x, x = a / (1 + a).
        // So to make the spacing even, we must shrink the outer padding with
        //     f(x) = x / (1 + x).
        // But we want to keep the outer padding, and make the inner gap bigger.
        // So we apply the inverse, f⁻¹ = -x / (x - 1) to the column gap.
        const gap = -g / (g - 1) * size

        if (flow === 'scrolled') {
            // FIXME: vertical-rl only, not -lr
            this.setAttribute('dir', vertical ? 'rtl' : 'ltr')
            this.#top.style.padding = '0'
            const columnWidth = maxInlineSize

            this.heads = null
            this.feet = null
            this.#header.replaceChildren()
            this.#footer.replaceChildren()

            this.columnCount = 1
            this.#replaceBackground()

            const layout = { width, height, flow, marginTop, marginRight, marginBottom, marginLeft, gap, columnWidth, columnCount: 1, rtl: this.#rtl }
            this.#lastLayout = layout
            return layout
        }

        const columnWidth = vertical
            ? (size / divisor - marginTop * 1.5 - marginBottom * 1.5)
            : (size / divisor - gap - marginRight / 2 - marginLeft / 2)
        // `dir` mirrors the horizontal scroll coordinates (negative scrollLeft
        // for RTL). Vertical books page along scrollTop, which never flips, so
        // an RTL writing mode must not reverse the host grid there.
        this.setAttribute('dir', this.#rtl && !vertical ? 'rtl' : 'ltr')

        // set background to `doc` background
        // this is needed because the iframe does not fill the whole element
        this.columnCount = divisor
        this.#replaceBackground()

        const marginalDivisor = vertical
            ? Math.min(2, Math.ceil(Math.floor(width) / Math.floor(maxInlineSize)))
            : divisor
        const marginalStyle = {
            gridTemplateColumns: `repeat(${marginalDivisor}, 1fr)`,
            gap: `${columnGap ?? gap}px`,
            direction: this.bookDir === 'rtl' ? 'rtl' : 'ltr',
        }
        Object.assign(this.#header.style, marginalStyle)
        Object.assign(this.#footer.style, marginalStyle)
        const heads = makeMarginals(marginalDivisor, 'head')
        const feet = makeMarginals(marginalDivisor, 'foot')
        this.heads = heads.map(el => el.children[0])
        this.feet = feet.map(el => el.children[0])
        this.#header.replaceChildren(...heads)
        this.#footer.replaceChildren(...feet)

        const layout = { width, height, marginTop, marginRight, marginBottom, marginLeft, gap, columnWidth, columnCount: divisor, columnGap, rtl: this.#rtl }
        this.#lastLayout = layout
        return layout
    }
    render() {
        if (this.#views.size === 0) return
        const primaryView = this.#primaryView
        if (!primaryView) return
        this.#stabilizing = true
        const layout = this.#beforeRender({
            vertical: this.#vertical,
            rtl: this.#rtl,
        })
        for (const [, view] of this.#views) {
            if (view.document) view.render(layout)
        }
        // Scroll synchronously to prevent visible layout shift during resize.
        // RAF deferral is only needed for initial display and mode switches
        // (handled by #display), not for resize re-renders.
        this.#scrollToAnchor(this.#anchor)
        this.#stabilizing = false
        this.dispatchEvent(new Event('stabilized'))
    }
    get scrolled() {
        return this.getAttribute('flow') === 'scrolled'
    }
    get noPreload() {
        return this.hasAttribute('no-preload')
    }
    get noBackground() {
        return this.hasAttribute('no-background')
    }
    get noContinuousScroll() {
        return this.scrolled && this.hasAttribute('no-continuous-scroll')
    }
    // The layered turn styles (slide/curl, readest#555) rasterize the outgoing
    // page with the View Transitions API; when the engine lacks it the caller
    // falls through to the push/two-phase animations, so old WebViews keep
    // working page turns.
    get #layeredTurn() {
        const style = this.getAttribute('turn-style')
        return (style === 'slide' || style === 'curl')
            && typeof document.startViewTransition === 'function'
            ? style : null
    }
    get scrollProp() {
        const { scrolled } = this
        return this.#vertical ? (scrolled ? 'scrollLeft' : 'scrollTop')
            : scrolled ? 'scrollTop' : 'scrollLeft'
    }
    get sideProp() {
        const { scrolled } = this
        return this.#vertical ? (scrolled ? 'width' : 'height')
            : scrolled ? 'height' : 'width'
    }
    get size() {
        return this.#container.getBoundingClientRect()[this.sideProp]
    }
    get viewSize() {
        const primaryView = this.#primaryView
        if (!primaryView) return 0
        return primaryView.element.getBoundingClientRect()[this.sideProp]
    }
    get start() {
        return this.#renderedStart - this.#getViewOffset(this.#primaryIndex)
    }
    get end() {
        return this.#renderedEnd - this.#getViewOffset(this.#primaryIndex)
    }
    get page() {
        return Math.floor(((this.start + this.end) / 2) / this.size)
    }
    get pages() {
        const primaryView = this.#primaryView
        if (!primaryView) return 0
        const viewSize = primaryView.element.getBoundingClientRect()[this.sideProp]
        return Math.round(viewSize / this.size)
    }
    get containerPosition() {
        return this.#container[this.scrollProp]
    }
    get isOverflowX() {
        return false
    }
    get isOverflowY() {
        return false
    }
    get #renderedViewSize() {
        if (this.#views.size === 0) return 0
        let total = 0
        for (const [, view] of this.#views)
            total += view.element.getBoundingClientRect()[this.sideProp]
        return total
    }
    get #renderedStart() {
        return Math.abs(this.#container[this.scrollProp])
    }
    get #renderedEnd() {
        return this.#renderedStart + this.size
    }
    get #renderedPage() {
        return Math.floor(((this.#renderedStart + this.#renderedEnd) / 2) / this.size)
    }
    get #renderedPages() {
        return Math.round(this.#renderedViewSize / this.size)
    }
    set containerPosition(newVal) {
        this.#container[this.scrollProp] = newVal
    }
    // Scroll offsets quantize to whole CSS pixels in both Blink and WebKit
    // (`scrollTop = 100.5` reads back 100 or 101), so a slow continuous scroll
    // such as Auto Scroll advances in visible one-pixel steps: at 5px/s that is
    // one jump every 200ms. The whole pixels stay in the scroll position, which
    // everything else reads; this carries only the sub-pixel remainder, as a
    // composited transform on the scrollport itself.
    //
    // The transform goes on #container rather than its children on purpose: a
    // transform on the children shifts their border boxes and shrinks the
    // container's scrollable overflow, which desynchronizes the scroll math
    // (readest#5663). Transforming the scrollport moves it as a whole and
    // leaves its scrollable overflow untouched.
    get subpixelOffset() {
        return this.#subpixelOffset
    }
    set subpixelOffset(offset) {
        const value = Number.isFinite(offset) ? offset : 0
        if (value === this.#subpixelOffset) return
        this.#subpixelOffset = value
        // Scrolling forward by `value` moves the content back by the same
        // amount. Vertical writing pages along the inline axis in scrolled mode.
        const axis = this.scrollProp === 'scrollLeft' ? 'X' : 'Y'
        this.#container.style.transform = value
            ? `translate${axis}(${-value}px) translateZ(0)` : ''
    }
    get scrollLocked() {
        return this.#scrollLocked
    }
    set scrollLocked(value) {
        this.#scrollLocked = value
    }

    scrollBy(dx, dy) {
        // #scrollBounds is populated by #scrollToPage and stays unset until
        // the first page settles. A swipe that lands before that happens
        // (for example a fast swipe right after the reader mounts, or
        // before a section has finished loading) would otherwise blow up
        // on the destructuring below — bail out and let the next settled
        // scroll re-enable swipe-driven motion.
        if (!this.#scrollBounds) return
        const delta = this.#vertical ? dy : dx
        const [offset, a, b] = this.#scrollBounds
        // RTL flips the forward/backward allowances only on the horizontal
        // scroll axis (negative scrollLeft); vertical books page along
        // scrollTop where forward is always positive.
        const rtl = this.#rtl && !this.#vertical
        const min = rtl ? offset - b : offset - a
        const max = rtl ? offset + a : offset + b
        this.containerPosition = Math.max(min, Math.min(max,
            this.containerPosition + delta))
    }

    // vx, vy: velocity at the end of the swipe (pixels per ms)
    // dx, dy: total distance swiped
    // dt: total time of the swipe (ms)
    snap(vx, vy, dx, dy, dt) {
        // Same guard as scrollBy: an early swipe whose touchend fires
        // before the first #scrollToPage seeds #scrollBounds would crash
        // on the destructuring. Skip the snap; the next settled scroll
        // populates the bounds and subsequent swipes work normally.
        if (!this.#scrollBounds) return
        // Page-turn swipes are horizontal in every writing mode: vertical-rl
        // books turn pages right-to-left like printed Japanese books
        // (readest#624), vertical-lr left-to-right. Vertical gestures belong
        // to the host's toolbar toggle, including a sideways lift-off flick.
        // Settle any horizontal drag started before the gesture turned vertical.
        if (this.#vertical && Math.abs(dx) <= Math.abs(dy)) {
            this.#settleDrag()
            return
        }
        const horizontal = Math.abs(vx) * 2 > Math.abs(vy)
        const pages = this.#renderedPages
        let page
        if (this.#vertical && !this.#layeredTurn
            && this.hasAttribute('animated') && !this.hasAttribute('eink')) {
            // Drag-follow gestures on vertical books (readest#624): the views
            // tracked the finger, so judge the turn like a paged carousel by
            // where the drag ended plus the release flick, instead of the
            // displacement heuristic below (which over-commits once content
            // visibly follows the finger).
            const width = this.#container.getBoundingClientRect().width
            const forwardSign = this.#rtl ? 1 : -1
            const dragged = this.#dragTranslateX
            // Flick direction in translate space: a rightward finger (vx < 0)
            // drags the views right.
            const flick = Math.abs(vx) > 0.3 ? -Math.sign(vx) : 0
            let turn
            if (Math.abs(dragged) > width / 2) {
                // Past halfway: commit unless flicked back the other way.
                turn = flick === -Math.sign(dragged) ? 0 : Math.sign(dragged)
            } else if (flick && (!dragged || flick === Math.sign(dragged))) {
                turn = flick
            } else {
                turn = 0
            }
            page = this.#renderedPage + turn * forwardSign
        } else {
            const velocity = vx
            const avgVelocity = dx / dt
            // Without drag-follow (eink, animation off,
            // layered turn styles) the scroll position never moves with the
            // finger; judge the whole gesture by displacement (avgVelocity)
            // like the eink path.
            const snapping = this.hasAttribute('animated') && !this.hasAttribute('eink')
                && !this.#vertical && !this.#layeredTurn
            // Drag-follow releases are judged by the release flick, so their
            // alignment uses the flick (last-sample) velocities. Displacement-
            // judged releases weigh the WHOLE gesture and their alignment must
            // too: the last-sample ratio is lift-off jitter — a vertical swipe
            // whose finger hooks sideways in its final milliseconds read as
            // horizontal, and the displacement heuristic amplified the tiny
            // net x-drift into a random page turn (layered slide on Android).
            const aligned = snapping ? horizontal : Math.abs(dx) > Math.abs(dy)
            // Horizontal swipes advance against the page progression.
            const sign = this.#rtl ? -1 : 1
            const [offset, a, b] = this.#scrollBounds
            const size = this.size
            const start = this.#renderedStart
            const end = this.#renderedEnd
            const min = Math.abs(offset) - a
            const max = Math.abs(offset) + b
            const v =  snapping ? velocity : avgVelocity
            const d = v * sign * size * (aligned ? 1 : 0)
            const velocityBased = isNaN(d) ? 0 : snapping ? d * 2 : d * 10
            // A slow but deliberate drag still turns the page. The drag-follow
            // release above is judged by its flick velocity alone (`d * 2`), so
            // a finger that drags most of a page across and then eases to a
            // stop before lifting scores ~0 and springs back: the page visibly
            // followed the finger, yet nothing commits, and the reader has to
            // flick hard to turn. Judge that release by the displacement the
            // reader actually saw. Alignment uses the displacement ratio, not
            // `aligned`, for the same reason it does on the displacement path:
            // at low speed the lift-off velocities are jitter.
            const distanceBased = snapping
                && Math.abs(dx) > Math.abs(dy)
                && Math.abs(dx) > size * DRAG_TURN_DISTANCE_RATIO
                ? Math.sign(dx) * sign * size
                : 0
            // Whichever offset commits more strongly wins, so a fast flick
            // keeps its existing multi-page reach.
            const snapOffset = Math.abs(velocityBased) > Math.abs(distanceBased)
                ? velocityBased : distanceBased
            page = Math.floor(Math.max(min, Math.min(max, (start + end) / 2 + snapOffset)) / size)
        }
        const dir = page < 0 ? -1 : page >= pages ? 1 : null
        const doGoTo = () => {
            if (!dir) return
            const sorted = this.#sortedViews
            const edgeIndex = dir < 0
                ? sorted[0]?.[0] ?? this.#primaryIndex
                : sorted[sorted.length - 1]?.[0] ?? this.#primaryIndex
            return this.#goTo({
                index: this.#adjacentIndex(dir, edgeIndex),
                anchor: dir < 0 ? () => 1 : () => 0,
            })
        }
        // Out of range — skip animation, go straight to adjacent section
        if (dir) {
            if (this.#vertical) {
                const sorted = this.#sortedViews
                const edgeIndex = dir < 0
                    ? sorted[0]?.[0] ?? this.#primaryIndex
                    : sorted[sorted.length - 1]?.[0] ?? this.#primaryIndex
                // Book boundary: nowhere to go, settle the drag back.
                if (this.#adjacentIndex(dir, edgeIndex) == null) return this.#settleDrag()
                this.#settleDrag(true)
            }
            return doGoTo()
        }
        this.#scrollToPage(page, 'snap')
    }
    #onTouchStart(e) {
        const previousState = this.#touchState
        const replacementTouch = Boolean(previousState?.active)
        if (replacementTouch) this.#rejectLayeredGesture(previousState)
        const multiTouch = e.touches.length > 1
        const touch = e.changedTouches[0]
        const currentTarget = e.currentTarget
        const isInnerDocument = currentTarget?.nodeType === 9
        const bounds = isInnerDocument
            ? { left: 0, width: currentTarget.documentElement?.clientWidth ?? 0 }
            : this.getBoundingClientRect()
        const localX = (touch?.clientX ?? 0) - bounds.left
        // Hosts can reserve a left-side control strip (for example, a vertical
        // brightness gesture) without disabling horizontal pagination there.
        // Only the low-slop fast paths are withheld; the normal fallback stays.
        const reservedLeftRatio = Math.max(0, Math.min(0.5,
            Number(this.getAttribute('turn-gesture-left-inset')) || 0))
        const earlyClaimBlocked = bounds.width > 0
            && localX <= bounds.width * reservedLeftRatio
        const edgeDirection = bounds.width > 0
            ? !earlyClaimBlocked && localX <= bounds.width * LAYERED_EDGE_REGION ? -1
                : localX >= bounds.width * (1 - LAYERED_EDGE_REGION) ? 1 : 0
            : 0
        const blocked = Boolean(multiTouch || this.#vtFinishing || this.#vtProgrammatic)
        this.#touchState = {
            x: touch?.screenX, y: touch?.screenY,
            t: e.timeStamp,
            vx: 0, xy: 0,
            dx: 0, dy: 0,
            dt: 0,
            releaseSamples: [{ distance: 0, time: e.timeStamp }],
            lastMovementTime: e.timeStamp,
            active: true,
            blocked,
            layeredGesture: blocked ? 'rejected' : 'pending',
            layeredEarlyClaimBlocked: earlyClaimBlocked,
            layeredEdgeDirection: edgeDirection,
            layeredHorizontalDirection: 0,
            layeredHorizontalSamples: 0,
            layeredHorizontalSampleTime: null,
        }
        if (replacementTouch || multiTouch) this.#touchScrolled = false
        // Hint to browser that scrolling will occur for better GPU layer management
        const pv = this.#primaryView
        if (pv?.element) {
            pv.element.style.willChange = 'transform'
        }
        // A touch on a vertical book takes over any in-flight page-turn slide
        // or settle: freeze the views where they are and let the drag continue
        // from that offset. Also re-syncs the drag offset to the rendered
        // transform so a stale value can never leak into the next gesture.
        if (this.#vertical && !this.scrolled && !this.#layeredTurn) {
            this.#slideTurnId++
            this.#isAnimating = false
            const children = [...this.#container.children]
            const transform = children[0] && getComputedStyle(children[0]).transform
            const m41 = transform && transform !== 'none' ? new DOMMatrix(transform).m41 : 0
            this.#dragTranslateX = m41
            for (const el of children) {
                if (!m41 && !el.style.transform && !el.style.transition) continue
                el.style.transition = 'none'
                el.style.transform = m41 ? `translateX(${m41}px)` : ''
            }
        }
        // Snapshot the invariant background paint inputs for the whole drag. The
        // layout is settled at the start of a gesture and only the scroll offset
        // changes while the finger moves, so every per-move #replaceBackground()
        // reuses this instead of forcing a fresh style+layout read each frame
        // (readest#4785). Cleared in #onTouchEnd; the snap that follows rebuilds
        // its own.
        this.#bgAnimContext = this.scrolled ? null : this.#computePaginatedBgContext()
    }
    #onTouchMove(e) {
        const state = this.#touchState
        if (!state?.active || state.blocked) return
        if (e.touches.length > 1) {
            if (this.#touchScrolled) e.preventDefault()
            this.#rejectLayeredGesture(state)
            return
        }
        if (state.pinched) {
            this.#rejectLayeredGesture(state)
            return
        }
        state.pinched = globalThis.visualViewport.scale > 1
        if (state.pinched) {
            this.#rejectLayeredGesture(state)
            return
        }
        if (this.scrolled) return
        // When the host opts out of swipe-to-paginate, let touch events reach
        // native behavior (text selection, etc.) without us tracking or
        // pre-empting them.
        if (this.hasAttribute('no-swipe')) {
            this.#rejectLayeredGesture(state)
            return
        }
        const doc = this.#primaryView?.document
        const selection = doc?.getSelection()
        if (selection && selection.rangeCount > 0 && !selection.isCollapsed) {
            this.#rejectLayeredGesture(state)
            return
        }
        const touch = e.changedTouches[0]
        const isStylus = touch.touchType === 'stylus'
        if (!isStylus) e.preventDefault()
        if (this.#scrollLocked) {
            this.#rejectLayeredGesture(state)
            return
        }
        const x = touch.screenX, y = touch.screenY
        const dx = state.x - x, dy = state.y - y
        const dt = e.timeStamp - state.t
        state.x = x
        state.y = y
        state.t = e.timeStamp
        state.vx = dx / dt
        state.vy = dy / dt
        state.dx += dx
        state.dy += dy
        state.dt += dt
        updateReleaseSample(state, state.dx, e.timeStamp)
        this.#touchScrolled = true
        if (!this.hasAttribute('animated') || this.hasAttribute('eink')) return
        // Layered turn styles track the finger by scrubbing a paused view
        // transition: the outgoing snapshot follows the drag over the still
        // incoming page, then commits or reverses on release.
        if (this.#layeredTurn) {
            if (!this.#vtDrag) this.#layeredDragStart(state, dx, dy)
            const drag = this.#vtDrag
            if (drag) {
                // Net finger travel along the forward direction (rtl books
                // advance with a rightward finger).
                const along = this.#rtl ? -state.dx : state.dx
                const totalDistance = drag.forward ? along : -along
                // Slide begins visually flat at the point where the gesture
                // arena awards ownership. The consumed recognition distance
                // still contributes to release intent below, but no longer
                // appears as a first-frame jump. Curl preserves its existing
                // touchstart-relative fold.
                const visualDistance = drag.style === 'slide'
                    ? totalDistance - drag.visualOriginDistance
                    : totalDistance
                drag.progress = Math.max(0, Math.min(1,
                    visualDistance / drag.width))
                this.#vtDragScrub()
            }
            return
        }
        if (!this.#vertical && Math.abs(state.dx) >= Math.abs(state.dy) && !this.hasAttribute('eink') && (!isStylus || Math.abs(dx) > 1)) {
            this.scrollBy(dx, 0)
        } else if (this.#vertical && Math.abs(state.dx) >= Math.abs(state.dy) && (!isStylus || Math.abs(dx) > 1)) {
            // Vertical books track a horizontal finger by translating the
            // views sideways: their pages stack along the vertical scroll
            // axis, so the scroll offset itself cannot follow the finger
            // (readest#624). The turn commits or settles on release in snap().
            this.#dragBy(dx)
        }
    }
    // Prepare the document for a layered page turn: choreography classes on
    // the root and the view-transition-name on the turn boundary. The host
    // app can mark a wider boundary with [data-view-transition-root] (e.g.
    // a wrapper that also contains its page header and footer, so the
    // furniture turns with the page in both layers); otherwise the outermost
    // shadow host in the document tree is named, since shadow-internal names
    // are tree-scoped away from the document-level pseudo selectors.
    #vtSetup(style, forward, scrubbing = false) {
        injectViewTransitionStyles()
        const html = document.documentElement
        const side = this.#rtl ? 'right' : 'left'
        // Which side the curl fold consumes the old page from: the outer
        // edge going forward, the spine going backward.
        const eatSide = (forward !== this.#rtl) ? 'right' : 'left'
        const classes = ['foliate-vt', `foliate-vt-${style}`,
            forward ? 'foliate-vt-forward' : 'foliate-vt-backward',
            `foliate-vt-${side}`, `foliate-vt-eat-${eatSide}`]
        if (scrubbing) classes.push('foliate-vt-scrub')
        let namedHost = this
        while (namedHost.getRootNode() instanceof ShadowRoot) {
            namedHost = namedHost.getRootNode().host
        }
        namedHost = namedHost.closest('[data-view-transition-root]') ?? namedHost
        namedHost.style.viewTransitionName = 'foliate-turn'
        this.#vtNamedHost = namedHost
        // Back the turn layers with the page colour: snapshots of books or
        // themes without an opaque background (e.g. textured themes) would
        // otherwise blend the two pages instead of occluding.
        const doc = this.#primaryView?.document
        const themeBg = doc?.documentElement
            ? doc.defaultView.getComputedStyle(doc.documentElement)
                .getPropertyValue('--theme-bg-color').trim()
            : ''
        html.style.setProperty('--foliate-vt-bg', themeBg || 'Canvas')
        html.classList.remove(...VIEW_TRANSITION_CLASSES)
        html.classList.add(...classes)
        return namedHost
    }
    #vtCleanup() {
        const html = document.documentElement
        html.classList.remove(...VIEW_TRANSITION_CLASSES)
        html.style.removeProperty('--foliate-vt-bg')
        this.#vtNamedHost?.style.removeProperty('view-transition-name')
        this.#vtNamedHost = null
    }
    // Permanently yield this touch sequence to another gesture owner. If the
    // layered snapshot already exists, cancel it through the normal lifecycle
    // instead of letting later samples (or a replacement finger) scrub it.
    #rejectLayeredGesture(state = this.#touchState) {
        if (state) {
            state.layeredGesture = 'rejected'
            state.layeredHorizontalDirection = 0
            state.layeredHorizontalSamples = 0
            state.layeredHorizontalSampleTime = null
        }
        const drag = this.#vtDrag
        if (!drag) return
        this.#vtDrag = null
        this.#finishLayeredDrag(drag, false)
    }
    // Begin a finger-tracked layered turn (readest#555). Edge-originated
    // gestures can claim on their first clear inward move. In the middle, two
    // consecutive horizontal samples can claim after 6px; a vertical gesture
    // locks out the turn before a landing wobble can become a page animation.
    // Ambiguous trajectories retain the established 24px + 1.5x fallback.
    // Once claimed, the existing `before-capture` lifecycle event explicitly
    // transfers ownership to the layered turn.
    #layeredDragStart(state, dx, dy) {
        if (this.#vtDrag || this.#vtFinishing || this.#vtProgrammatic || !this.#scrollBounds) return
        const style = this.#layeredTurn
        if (!style) return
        if (state.layeredGesture !== 'pending') return

        const absDx = Math.abs(state.dx)
        const absDy = Math.abs(state.dy)
        if (absDy >= LAYERED_VERTICAL_REJECT_PX && absDy > absDx) {
            state.layeredGesture = 'rejected'
            return
        }

        const direction = Math.sign(dx)
        const locallyHorizontal = direction !== 0 && Math.abs(dx) > Math.abs(dy)
        const clearlyHorizontal = locallyHorizontal
            && Math.abs(dx) >= Math.abs(dy) * LAYERED_FALLBACK_DOMINANCE
        const cumulativelyHorizontal = absDx
            >= absDy * LAYERED_FALLBACK_DOMINANCE
        if (locallyHorizontal && cumulativelyHorizontal) {
            const recentSample = state.layeredHorizontalSampleTime != null
                && state.t - state.layeredHorizontalSampleTime
                    <= LAYERED_EARLY_SAMPLE_INTERVAL_MS
            if (state.layeredHorizontalDirection === direction && recentSample) {
                state.layeredHorizontalSamples++
            } else {
                state.layeredHorizontalDirection = direction
                state.layeredHorizontalSamples = 1
            }
            state.layeredHorizontalSampleTime = state.t
        } else {
            state.layeredHorizontalDirection = 0
            state.layeredHorizontalSamples = 0
            state.layeredHorizontalSampleTime = null
        }

        const edgeClaim = state.layeredEdgeDirection !== 0
            && !state.layeredEarlyClaimBlocked
            && direction === state.layeredEdgeDirection
            && Math.sign(state.dx) === state.layeredEdgeDirection
            && clearlyHorizontal
            && cumulativelyHorizontal
        const earlyCenterClaim = state.layeredEdgeDirection === 0
            && !state.layeredEarlyClaimBlocked
            && state.layeredHorizontalSamples >= 2
            && absDx >= LAYERED_EARLY_CLAIM_PX
            && Math.sign(state.dx) === state.layeredHorizontalDirection
        const fallbackClaim = absDx >= LAYERED_FALLBACK_CLAIM_PX
            && absDx >= absDy * LAYERED_FALLBACK_DOMINANCE
        if (!edgeClaim && !earlyCenterClaim && !fallbackClaim) return

        // Finger travel along the forward direction decides which neighbor
        // page gets snapshotted.
        const along = this.#rtl ? -state.dx : state.dx
        const forward = along > 0
        state.layeredGesture = 'claimed'
        // Gesture ownership is independent of whether an adjacent page exists.
        // At a book boundary the turn cannot start, but the host must still
        // suppress the browser's synthesized click for this horizontal drag.
        // Keep this separate from layered-turn-state so the established
        // snapshot lifecycle remains unchanged.
        this.dispatchEvent(new CustomEvent('layered-turn-gesture-claimed', {
            detail: { style, forward },
        }))
        const pages = this.#renderedPages
        const target = this.#renderedPage + (forward ? 1 : -1)
        if (target < 0 || target >= pages) return
        const offset = this.size * (this.#rtl && !this.#vertical ? -target : target)
        ++this.#slideTurnId
        this.#isAnimating = true
        this.dispatchEvent(new CustomEvent('layered-turn-state', {
            detail: { phase: 'before-capture', style, forward },
        }))
        const startPosition = this.containerPosition
        let turnRoot
        let transition
        try {
            turnRoot = this.#vtSetup(style, forward, true)
            transition = document.startViewTransition(() => {
                this.containerPosition = offset
                if (!this.scrolled) {
                    this.#bgAnimContext = null
                    this.#replaceBackground()
                }
                // The old snapshot now owns the visible toolbar. Let the host hide
                // the live copy synchronously before the new snapshot is captured,
                // so its regular opacity transition cannot run beside the page.
                this.dispatchEvent(new CustomEvent('layered-turn-state', {
                    detail: { phase: 'covered', style, forward },
                }))
            })
        } catch {
            // A synchronous setup/capture failure must release both the global
            // View Transition styling and the host's before-capture ownership.
            // Reject the rest of this touch so touchend cannot reinterpret it
            // as a legacy snap after the layered lifecycle has already ended.
            state.layeredGesture = 'rejected'
            this.containerPosition = startPosition
            this.#vtCleanup()
            this.#isAnimating = false
            this.dispatchEvent(new CustomEvent('layered-turn-state', {
                detail: { phase: 'finished', style, forward, committed: false },
            }))
            return
        }
        const drag = {
            transition, offset, startPosition, forward,
            style, progress: 0, anims: null,
            visualOriginDistance: style === 'slide'
                ? Math.max(0, forward ? along : -along) : 0,
            // Progress must use the width of the actual named snapshot. The
            // inner content container can be narrower because of page margins;
            // using it makes the sheet gradually outrun the finger.
            width: turnRoot.getBoundingClientRect().width
                || this.#container.getBoundingClientRect().width,
        }
        this.#vtDrag = drag
        transition.ready.then(() => {
            if (this.#vtDrag !== drag && this.#vtFinishing !== drag) return
            const anims = document.getAnimations().filter(a =>
                a.effect?.pseudoElement?.includes('(foliate-turn)'))
            for (const a of anims) {
                // CSS is authoritative for linear scrubbing. Keep this as a
                // best-effort fallback for engines that expose mutable pseudo
                // animation effects.
                try { a.effect.updateTiming({ easing: 'linear' }) } catch { /* UA animation */ }
                a.pause()
            }
            drag.anims = anims
            this.#vtDragScrub(drag)
            this.dispatchEvent(new CustomEvent('layered-turn-state', {
                detail: { phase: 'ready', style, forward },
            }))
        }, () => {
            // Capture failed or was skipped; release falls back to snap().
            if (this.#vtDrag === drag || this.#vtFinishing === drag) drag.failed = true
        })
    }
    #vtDragScrub(drag = this.#vtDrag) {
        if (!drag?.anims) return
        for (const a of drag.anims) {
            const duration = a.effect.getTiming().duration
            a.currentTime = drag.progress * 0.999
                * (typeof duration === 'number' ? duration : 300)
        }
    }
    // Resolve a finger-tracked layered turn: play the paused animations to
    // the end to commit, or reverse them and put the live content back to
    // cancel. The scroll offset already sits on the target page during the
    // drag (the snapshot hides it), so cancel restores it under the overlay
    // before the transition is skipped.
    async #finishLayeredDrag(drag, commit, playbackRate = 1) {
        if (this.#vtFinishing === drag) return
        this.#vtFinishing = drag
        const { transition, offset, startPosition, style, forward } = drag
        const { size } = this
        const id = ++this.#slideTurnId
        this.#isAnimating = true
        try {
            // The update callback owns the old/new snapshot boundary. Await it
            // before any terminal event so `covered` can never arrive after a
            // very fast release has already announced cancellation.
            try { await transition.updateCallbackDone } catch { /* skipped */ }
            try { await transition.ready } catch { /* capture failed */ }
            if (id !== this.#slideTurnId) return

            const anims = drag.anims
            if (anims) for (const a of anims) updatePlaybackRate(a, playbackRate)
            if (commit) {
                if (anims) for (const a of anims) a.play()
                try {
                    await transition.finished
                } catch { /* skipped */ }
            } else {
                if (anims) {
                    for (const a of anims) a.reverse()
                    try {
                        await Promise.all(anims.map(a => a.finished))
                    } catch { /* superseded */ }
                }
                if (id !== this.#slideTurnId) return
                // Restore the pre-turn page under the overlay, then drop it.
                this.containerPosition = startPosition
                this.dispatchEvent(new CustomEvent('layered-turn-state', {
                    detail: { phase: 'cancelled', style, forward, committed: false },
                }))
                // Give the host two rendering opportunities to paint restored
                // chrome underneath the now-flat snapshot before it is removed.
                await new Promise(resolve => requestAnimationFrame(() =>
                    requestAnimationFrame(resolve)))
                if (id !== this.#slideTurnId) return
                try { transition.skipTransition() } catch { /* already done */ }
                try {
                    await transition.finished
                } catch { /* skipped */ }
            }
            if (id !== this.#slideTurnId) return
            this.#vtCleanup()
            this.#isAnimating = false
            const finalPosition = commit ? offset : startPosition
            this.containerPosition = finalPosition
            this.#scrollBounds = [
                finalPosition,
                this.atStart ? 0 : size,
                this.atEnd ? 0 : size,
            ]
            this.#afterScroll('snap')
            this.dispatchEvent(new CustomEvent('layered-turn-state', {
                detail: { phase: 'finished', style, forward, committed: commit },
            }))
        } finally {
            if (this.#vtFinishing === drag) this.#vtFinishing = null
        }
    }
    #dragBy(dx) {
        if (!this.#scrollBounds) return
        const [, a, b] = this.#scrollBounds
        const width = this.#container.getBoundingClientRect().width
        // Exit direction of a forward turn: vertical-rl pages exit right.
        const forwardSign = this.#rtl ? 1 : -1
        const max = (forwardSign > 0 ? b : a) > 0 ? width : 0
        const min = (forwardSign > 0 ? a : b) > 0 ? -width : 0
        this.#dragTranslateX = Math.max(min, Math.min(max, this.#dragTranslateX - dx))
        for (const el of this.#container.children) {
            el.style.transition = 'none'
            el.style.transform = `translateX(${this.#dragTranslateX}px)`
        }
    }
    // Animate a released drag back to rest when the page did not turn, and
    // drop the inline drag styles once the views are back in place. Pass
    // instant=true to drop them without the settle animation.
    #settleDrag(instant) {
        const startX = this.#dragTranslateX
        this.#dragTranslateX = 0
        const children = [...this.#container.children]
        if (!children.some(el => el.style.transform)) return
        const cleanup = () => {
            for (const el of children) {
                el.style.willChange = ''
                el.style.transition = ''
                el.style.transform = ''
            }
        }
        if (!startX || instant) return cleanup()
        const id = ++this.#slideTurnId
        for (const el of children) {
            el.style.transition = 'transform 150ms ease-out'
            el.style.transform = 'translateX(0px)'
        }
        setTimeout(() => {
            if (id !== this.#slideTurnId) return
            cleanup()
        }, 170)
    }
    #onTouchEnd(e) {
        // Remove will-change hint to free GPU resources
        // if (this.#view?.element) {
        //     this.#view.element.style.willChange = 'auto'
        // }

        // The drag is over: drop the drag-time paint snapshot so the snap
        // animation (or any other repaint) rebuilds a fresh context.
        this.#bgAnimContext = null
        const state = this.#touchState
        if (state) state.active = false
        if (state?.blocked) {
            this.#touchScrolled = false
            return
        }

        if (!this.#touchScrolled) {
            // A tap that never dragged may still have taken over a mid-flight
            // transform in #onTouchStart; put the views back to rest.
            if (this.#vertical && !this.scrolled && this.#dragTranslateX) this.#settleDrag()
            return
        }
        this.#touchScrolled = false
        if (this.scrolled) return
        if (this.hasAttribute('no-swipe')) return
        const layeredRejected = this.#layeredTurn
            && state?.layeredGesture === 'rejected'
        // Vertical books still need snap() to settle any horizontal drag.
        if (layeredRejected && !this.#vertical) return

        // A finger that rested before lifting has no flick momentum; the
        // last touchmove velocity is stale by the rest duration.
        if (state && e && e.timeStamp - state.t > RELEASE_PAUSE_THRESHOLD_MS) {
            state.vx = 0
            state.vy = 0
        }
        const releaseTouch = e?.changedTouches?.[0]
        let releaseDx = state?.dx ?? 0
        let releaseDy = state?.dy ?? 0
        if (state && releaseTouch) {
            // A quick lift can carry the final sample only in changedTouches.
            // Use that point consistently for Slide's velocity, progress, and
            // whole-gesture horizontal-intent guard.
            releaseDx += state.x - releaseTouch.screenX
            releaseDy += state.y - releaseTouch.screenY
        }
        // Also stamp an unchanged final position. Besides making a deliberate
        // pause velocity-free, this is defensive against non-standard UAs
        // that omit touchend.changedTouches.
        if (state) updateReleaseSample(state, releaseDx, e.timeStamp)

        // A finger-tracked layered turn resolves here. Slide projects recent
        // release velocity onto its current progress; Curl keeps the existing
        // last-move flick-or-halfway rule. A page-turn commit also requires
        // the WHOLE gesture to be predominantly horizontal: a finger
        // landing with a sideways wobble can start the drag before any
        // vertical distance accumulates, and the lift-off flick velocity is
        // jitter — judged alone they turned the page randomly on vertical
        // toolbar-toggle swipes (Android WebView report).
        const drag = this.#vtDrag
        if (drag) {
            this.#vtDrag = null
            // Keep Curl's established whole-gesture guard exactly as-is;
            // Slide uses the actual lift-off point required by its projection.
            const gestureDx = drag.style === 'slide' ? releaseDx : (state?.dx ?? 0)
            const gestureDy = drag.style === 'slide' ? releaseDy : (state?.dy ?? 0)
            const gestureAligned = state
                ? Math.abs(gestureDx) > Math.abs(gestureDy) : true
            const alongV = this.#rtl ? -(state?.vx ?? 0) : (state?.vx ?? 0)
            const recentVx = state ? getReleaseVelocity(state) : 0
            const recentAlongV = this.#rtl ? -recentVx : recentVx
            const progressVelocity = recentAlongV * (drag.forward ? 1 : -1)
            const releaseAlong = this.#rtl ? -releaseDx : releaseDx
            const releaseDistance = drag.forward ? releaseAlong : -releaseAlong
            const releaseProgress = Math.max(0, Math.min(1,
                releaseDistance / drag.width))
            const releaseVisualProgress = Math.max(0, Math.min(1,
                (releaseDistance - drag.visualOriginDistance) / drag.width))
            const projectedProgress = releaseProgress
                + progressVelocity * SLIDE_RELEASE_PROJECTION_MS / drag.width
            const curlFlick = Math.abs(alongV) > 0.3
                ? Math.sign(alongV) * (drag.forward ? 1 : -1) : 0
            const commit = gestureAligned && (drag.style === 'slide'
                ? projectedProgress > 0.5
                : curlFlick > 0 ? true : curlFlick < 0 ? false : drag.progress > 0.5)
            // Do not scrub a gesture that ended vertically: flashing its final
            // horizontal component here would defeat the intent guard. A valid
            // Slide release settles from the actual lift-off position; Curl's
            // existing progress and commit mapping remain unchanged.
            if (gestureAligned && drag.style === 'slide') {
                drag.progress = releaseVisualProgress
                this.#vtDragScrub(drag)
            }
            const targetDirection = (drag.forward ? 1 : -1) * (commit ? 1 : -1)
            // Settle pacing uses a short release window; the decision above
            // uses it for the lighter Slide gesture while Curl keeps its
            // original last-sample commit rule.
            const releaseSpeed = recentAlongV * targetDirection
            const playbackRate = layeredSettlePlaybackRate(drag.style, releaseSpeed)
            this.#finishLayeredDrag(drag, commit, playbackRate)
            return
        }

        // XXX: Firefox seems to report scale as 1... sometimes...?
        // at this point I'm basically throwing `requestAnimationFrame` at
        // anything that doesn't work
        const snapState = state
        requestAnimationFrame(() => {
            if (globalThis.visualViewport.scale === 1 && snapState
                && this.#touchState === snapState) {
                const { vx, vy, dx, dy, dt } = snapState
                // Direction ownership is final for this touch sequence. Once
                // vertical wins the layered arena, discard later horizontal
                // hooks; snap() settles any horizontal drag in vertical books.
                this.snap(layeredRejected ? 0 : vx, vy,
                    layeredRejected ? 0 : dx, dy, dt)
            }
        })
    }
    #onTouchCancel() {
        this.#bgAnimContext = null
        const state = this.#touchState
        if (state) state.active = false
        if (state?.blocked) {
            this.#touchScrolled = false
            return
        }

        const drag = this.#vtDrag
        if (drag) {
            this.#vtDrag = null
            this.#touchScrolled = false
            this.#finishLayeredDrag(drag, false)
            return
        }

        const wasScrolled = this.#touchScrolled
        this.#touchScrolled = false
        if (this.scrolled || this.hasAttribute('no-swipe')) return
        if (this.#layeredTurn && state?.layeredGesture === 'rejected'
            && !this.#vertical) return
        if (this.#vertical) {
            this.#settleDrag()
        } else if (wasScrolled && this.#scrollBounds) {
            this.#scrollTo(this.#scrollBounds[0], 'snap')
        }
    }
    // allows one to process rects as if they were LTR and horizontal
    #getRectMapper(view) {
        if (this.scrolled) {
            const size = view ? view.element.getBoundingClientRect()[this.sideProp] : this.#renderedViewSize
            const marginTop = this.#marginTop
            const marginBottom = this.#marginBottom
            return this.#vertical
                ? ({ left, right }) =>
                    ({ left: size - right - marginTop, right: size - left - marginBottom })
                : ({ top, bottom }) => ({ left: top - marginTop, right: bottom - marginBottom })
        }
        // For RTL the mapper mirrors a rect within the iframe-local
        // coordinate space of the *target view* (each view is a separate
        // document with its own column layout), not across the whole
        // container. Using `#renderedPages * size` (= total width of all
        // loaded views) was correct only when a single view was loaded;
        // once #fillVisibleArea pre-loads adjacent sections the total
        // width grows but the per-view rect coordinates do not change,
        // so the mapper would scroll the same anchor to a different
        // (further-right) container offset on every re-anchor — driving
        // the page off the user's saved position. Use the supplied
        // view's width when available, falling back to the primary view.
        const targetView = view ?? this.#primaryView
        const viewSize = targetView
            ? targetView.element.getBoundingClientRect()[this.sideProp]
            : this.#renderedViewSize
        // Vertical books map the block axis (top/bottom) onto the scroll
        // axis regardless of page progression: vertical-rl is RTL but its
        // scrollTop still grows forward, so the RTL mirror below only
        // applies to horizontal writing.
        return this.#vertical
            ? ({ top, bottom }) => ({ left: top, right: bottom })
            : this.#rtl
                ? ({ left, right }) =>
                    ({ left: viewSize - right, right: viewSize - left })
                : f => f
    }
    async #scrollToRect(rect, reason) {
        if (this.scrolled) {
            // rect is in iframe-local coordinates; add view offset
            // to convert to container scroll coordinates
            const localOffset = this.#getRectMapper()(rect).left - 3
            const viewOffset = this.#getViewOffset(this.#primaryIndex)
            return this.#scrollTo(viewOffset + localOffset, reason)
        }
        // rect is in iframe-local coordinates. Convert to container
        // coordinates by adding the primary view's offset.
        const localOffset = this.#getRectMapper()(rect).left
        const viewOffset = this.#getViewOffset(this.#primaryIndex)
        const containerOffset = viewOffset + localOffset
        return this.#scrollToPage(Math.floor(containerOffset / this.size + 0.01), reason)
    }
    async #scrollTo(offset, reason, smooth) {
        const { size } = this
        // Near-equality, not exact: on fractional device-pixel-ratio screens
        // (e.g. 2.75) the container scroll rests a sub-pixel off the page
        // offset, and an exact check made every same-page settle miss this
        // short-circuit and run a full animation — with the layered turn
        // styles, a visible full-page view-transition flash on every
        // vertical toolbar-toggle swipe.
        if (Math.abs(this.containerPosition - offset) < 1) {
            this.#scrollBounds = [offset, this.atStart ? 0 : size, this.atEnd ? 0 : size]
            // A released drag that stays on the same page settles back to rest.
            if (this.#vertical && !this.scrolled) this.#settleDrag()
            this.#afterScroll(reason)
            return
        }
        // FIXME: vertical-rl only, not -lr
        if (this.scrolled && this.#vertical) offset = -offset
        if ((reason === 'snap' || smooth) && this.hasAttribute('animated') && !this.hasAttribute('eink')) {
            // Layered turn styles: snapshot the outgoing page and animate it
            // over the live, stationary incoming page (readest#555). Works
            // for every writing mode since the snapshot is axis-agnostic —
            // but only for actual page changes: a sub-page settle must not
            // snapshot and re-slide the page it is already resting on.
            const turning = Math.abs(offset - this.containerPosition) > size / 2
            const layered = !this.scrolled && turning ? this.#layeredTurn : null
            if (layered) return this.#viewTransitionTurn(offset, reason, layered)
            const startPosition = this.containerPosition
            this.#isAnimating = true
            // Vertical paginated books page along scrollTop but read
            // horizontally, so a scroll-axis slide would move perpendicular to
            // the page turn. Run the two-phase horizontal slide instead:
            // vertical-rl turns forward exit to the right (page progression),
            // vertical-lr to the left (readest#624).
            if (!this.scrolled && this.#vertical) {
                this.#bgAnimContext = null
                // Oversized sections would composite as one giant layer to
                // animate the transform (the freeze rafAnimateScroll exists to
                // avoid), and a native scroll animation cannot move
                // horizontally here, so swap instantly.
                if (!this.hasAttribute('gpu-composite')
                    && this.#renderedViewSize > RAF_ANIMATE_SCROLL_THRESHOLD) {
                    this.#isAnimating = false
                    this.#settleDrag(true)
                    this.containerPosition = offset
                    this.#scrollBounds = [offset, this.atStart ? 0 : size, this.atEnd ? 0 : size]
                    this.#afterScroll(reason)
                    return
                }
                const id = ++this.#slideTurnId
                const forward = offset > startPosition
                const exitSign = (this.#rtl ? 1 : -1) * (forward ? 1 : -1)
                const width = this.#container.getBoundingClientRect().width
                // Continue from a finger drag already in progress.
                const dragStartX = this.#dragTranslateX
                this.#dragTranslateX = 0
                return slideTurnAnimation(
                    this.#container, this.scrollProp, offset, exitSign, width, 300,
                    () => id !== this.#slideTurnId,
                    // Reposition the background segments for the new scroll
                    // offset while both pages are off-screen.
                    () => this.#replaceBackground(),
                    dragStartX,
                ).then(() => {
                    if (id !== this.#slideTurnId) return
                    this.#isAnimating = false
                    this.#scrollBounds = [offset, this.atStart ? 0 : size, this.atEnd ? 0 : size]
                    this.#afterScroll(reason)
                })
            }
            // Snapshot the invariant paint inputs once; every per-frame
            // #replaceBackground below reuses this instead of forcing a fresh
            // style+layout read each frame (readest#4785).
            this.#bgAnimContext = this.scrolled ? null : this.#computePaginatedBgContext()
            // For a large section the CSS-transform animation blocks the UI while
            // Blink composites the oversized layer; animate the native scroll
            // offset instead (incremental/tiled, like a swipe), keeping the
            // per-page backgrounds synced each frame. Hosts that composite large
            // layers without that freeze (Apple WebKit, via the gpu-composite
            // opt-in) skip this main-thread fallback and keep the smooth GPU
            // cssAnimateScroll path even for large sections (readest#4768).
            if (!this.hasAttribute('gpu-composite')
                && this.#renderedViewSize > RAF_ANIMATE_SCROLL_THRESHOLD) {
                return rafAnimateScroll(startPosition, offset, 300, easeOutQuad, x => {
                    this.#container[this.scrollProp] = x
                    if (!this.scrolled) this.#replaceBackground()
                }).then(() => {
                    this.#isAnimating = false
                    this.#bgAnimContext = null
                    this.#scrollBounds = [offset, this.atStart ? 0 : size, this.atEnd ? 0 : size]
                    this.#afterScroll(reason)
                })
            }
            // Slide the per-view backgrounds in lockstep with the content. The
            // content animates via a transform on each view; we re-sync the
            // backgrounds to that animated offset every frame so each page's
            // colour stays glued to its content as it slides. Pre-setting the
            // destination instead made the outgoing page lose its background the
            // instant the animation started, flashing the wrong colour across
            // the part of the screen it still covered until it slid off.
            if (!this.scrolled) {
                this.#replaceBackground(startPosition)
                const child = this.#container.children[0]
                const syncBackground = () => {
                    if (!this.#isAnimating) return
                    const transform = child && getComputedStyle(child).transform
                    const tx = transform && transform !== 'none'
                        ? new DOMMatrix(transform)[this.#vertical ? 'm42' : 'm41'] : 0
                    this.#replaceBackground(startPosition - tx)
                    requestAnimationFrame(syncBackground)
                }
                requestAnimationFrame(syncBackground)
            }
            // Use GPU-accelerated scroll animation for smoother experience on high refresh rate screens
            return cssAnimateScroll(
                this.#container,
                this.scrollProp,
                startPosition,
                offset,
                300,
            ).then(() => {
                this.#isAnimating = false
                this.#bgAnimContext = null
                this.#scrollBounds = [offset, this.atStart ? 0 : size, this.atEnd ? 0 : size]
                this.#afterScroll(reason)
            })
        } else {
            if (this.#vertical && !this.scrolled) this.#settleDrag(true)
            this.containerPosition = offset
            this.#scrollBounds = [offset, this.atStart ? 0 : size, this.atEnd ? 0 : size]
            this.#afterScroll(reason)
        }
    }
    // Turn the page with a View Transitions snapshot (readest#555): the live
    // content jumps to the destination inside the transition callback, then
    // the rasterized outgoing page slides away or curls open ON TOP of it, so
    // the incoming page stays perfectly still underneath (Apple Books style).
    // Forward turns move the old snapshot out; backward turns bring the new
    // snapshot in over the still page. The choreography is selected with
    // classes on the document root, where the ::view-transition pseudo tree
    // lives.
    async #viewTransitionTurn(offset, reason, style) {
        // A layered transition has exclusive ownership of the document-level
        // pseudo tree. Ignore overlapping navigation until its lifecycle has
        // reached cleanup; otherwise the newer generation strands the older
        // turn before it can dispatch `finished`.
        if (this.#vtDrag || this.#vtFinishing || this.#vtProgrammatic) return
        // An accepted keyboard/tap turn owns navigation for the remainder of
        // any finger that is still down. Permanently reject a pending arena
        // candidate so its later move cannot reuse the pre-turn start point
        // and launch a second layered turn after this transition finishes.
        if (this.#touchState?.active) this.#rejectLayeredGesture(this.#touchState)
        const { size } = this
        const startPosition = this.containerPosition
        // RTL horizontal scroll coordinates are negative; compare magnitudes.
        const forward = Math.abs(offset) > Math.abs(startPosition)
        const id = ++this.#slideTurnId
        this.#isAnimating = true
        this.#settleDrag(true)
        this.#vtSetup(style, forward)
        const transition = document.startViewTransition(() => {
            this.containerPosition = offset
            if (!this.scrolled) {
                this.#bgAnimContext = null
                this.#replaceBackground()
            }
        })
        const active = { transition, id }
        this.#vtProgrammatic = active
        try {
            try {
                await transition.finished
            } catch {
                // Interrupted or skipped; the newer turn owns the cleanup.
            }
            if (id !== this.#slideTurnId) return
            this.#vtCleanup()
            this.#isAnimating = false
            // A neighbor view finishing its load mid-transition re-anchors the
            // container to the stale pre-turn anchor; re-assert the destination
            // like the push animation does at its end.
            this.containerPosition = offset
            this.#scrollBounds = [offset, this.atStart ? 0 : size, this.atEnd ? 0 : size]
            this.#afterScroll(reason)
        } finally {
            if (this.#vtProgrammatic === active) this.#vtProgrammatic = null
        }
    }
    async #scrollToPage(page, reason, smooth) {
        // Negative offsets are an artifact of RTL horizontal scroll
        // coordinates; vertical books page along scrollTop, always positive.
        const offset = this.size * (this.#rtl && !this.#vertical ? -page : page)
        return this.#scrollTo(offset, reason, smooth)
    }
    async scrollToAnchor(anchor, select, smooth) {
        return this.#scrollToAnchor(anchor, select ? 'selection' : 'navigation', smooth)
    }
    async #scrollToAnchor(anchor, reason = 'anchor', smooth = false) {
        this.#anchor = anchor
        const rects = uncollapse(anchor)?.getClientRects?.()
        // if anchor is an element or a range
        if (rects) {
            // when the start of the range is immediately after a hyphen in the
            // previous column, there is an extra zero width rect in that column
            const rect = Array.from(rects)
                .find(r => r.width > 0 && r.height > 0 && r.x >= 0 && r.y >= 0) || rects[0]
            // An anchor can have no client rects at all: a range over content
            // that is entirely absolutely positioned (e.g. a Duokan fullscreen
            // cover pins its only image) has no in-flow boxes. Bailing without
            // scrolling would leave #scrollBounds unseeded, and scrollBy/snap
            // then silently drop every swipe on the page (#5263). Settle on
            // the primary section's start instead.
            if (!rect) return this.scrolled
                ? this.#scrollTo(this.#getViewOffset(this.#primaryIndex), reason)
                : this.#scrollToPage(this.#getPagesBeforeView(this.#primaryIndex), reason)
            await this.#scrollToRect(rect, reason)
            // focus the element when navigating with keyboard or screen reader
            if (reason === 'navigation') {
                let node = anchor.focus ? anchor : undefined
                if (!node && anchor.startContainer) {
                    node = anchor.startContainer
                    if (node.nodeType === Node.TEXT_NODE) {
                        node = node.parentElement
                    }
                }
                if (node && node.focus) {
                    node.tabIndex = -1
                    node.style.outline = 'none'
                    node.focus({ preventScroll: true })
                }
            }
            return
        }
        // if anchor is a fraction
        if (this.scrolled) {
            // In scrolled mode with multi-view, offset to the primary view's position
            const primaryOffset = this.#getViewOffset(this.#primaryIndex)
            const primaryView = this.#primaryView
            const primarySize = primaryView
                ? primaryView.element.getBoundingClientRect()[this.sideProp] : this.#renderedViewSize
            await this.#scrollTo(primaryOffset + anchor * primarySize, reason, smooth)
            return
        }
        // In paginated mode, account for pages before the primary section
        const primaryView = this.#primaryView
        if (!primaryView) return
        const pagesBeforePrimary = this.#getPagesBeforeView(this.#primaryIndex)
        const textPages = primaryView.contentPages
        // Same as the rect-less bail above: a section that measured zero
        // content pages must still settle so #scrollBounds gets seeded.
        if (!textPages) return this.#scrollToPage(pagesBeforePrimary, reason)
        // textPages is in column units; convert to spread page for scrolling
        const newColumn = Math.round(anchor * (textPages - 1))
        const newSpreadPage = Math.floor(newColumn / this.columnCount)
        await this.#scrollToPage(pagesBeforePrimary + newSpreadPage, reason, smooth)
    }
    // Get the pixel offset of a view within the container
    #getViewOffset(index) {
        let offset = 0
        for (const [i, view] of this.#sortedViews) {
            if (i === index) return offset
            offset += view.element.getBoundingClientRect()[this.sideProp]
        }
        return offset
    }
    // Get number of full pages (spreads) before a given view.
    // Uses floor so the view's first column is always on or after
    // the returned page — never rounded past it. The 0.01 tolerance
    // absorbs sub-pixel drift on fractional-DPR devices where
    // getBoundingClientRect() accumulates ~0.0001px errors.
    #getPagesBeforeView(index) {
        return Math.floor(this.#getViewOffset(index) / this.size + 0.01)
    }
    #getVisibleRange() {
        const targetView = this.#primaryView
        if (!targetView?.document) return
        const viewOffset = this.#getViewOffset(this.#primaryIndex)
        if (this.scrolled) {
            // In scrolled mode several sections can share the viewport at a
            // section boundary, and the primary view may even be scrolled out
            // of view. Prefer the view that covers the viewport centre — that
            // is the section the reader is actually reading, so its title is
            // the one to show. Falling back to the first overlapping view (the
            // old behaviour) would report a thin sliver at the top edge, whose
            // chapter title no longer matches the dominant content
            // (readest#4436). Keep that first valid range as a fallback for
            // when no loaded view covers the centre (e.g. at the very top or
            // bottom of the book).
            const center = this.#renderedStart + this.size / 2
            let fallback
            let collapsed
            for (const [index, v] of this.#sortedViews) {
                if (!v.document) continue
                const off = this.#getViewOffset(index)
                const vSize = v.element.getBoundingClientRect()[this.sideProp]
                // Skip views entirely outside the viewport
                if (off + vSize <= this.#renderedStart || off >= this.#renderedEnd) continue
                const range = getVisibleRange(v.document,
                    this.#renderedStart - off, this.#renderedEnd - off,
                    this.#getRectMapper(v))
                if (!range) continue
                const coversCenter = center >= off && center < off + vSize
                // A section with no accepted node in view (an image-only cover
                // whose <svg>/<img> is clipped by the viewport, a text-less
                // background page) collapses onto <body>. Prefer any view with
                // real content, but keep the collapsed range as a last resort:
                // dropping it left an image-only cover with no relocate at all
                // on open, so the host never got a location to record or sync
                // reading progress from. Paginated mode relocates on the same
                // collapsed range.
                if (range.collapsed) {
                    if (!collapsed || coversCenter) collapsed = { range, index }
                    continue
                }
                if (coversCenter) return { range, index }
                fallback ??= { range, index }
            }
            return fallback ?? collapsed
        }
        const range = getVisibleRange(targetView.document,
            this.#renderedStart - viewOffset,
            this.#renderedEnd - viewOffset,
            this.#getRectMapper(targetView))
        return range ? { range, index: this.#primaryIndex } : undefined
    }
    // Whether the current Range anchor starts inside the visible range.
    // Fraction and Element anchors are never considered visible, so they
    // keep being replaced by the visible range as before.
    #anchorIsVisible(range) {
        const anchor = this.#anchor
        if (!anchor?.startContainer) return false
        const node = anchor.startContainer
        // comparePoint throws for a node rooted outside the range's document,
        // i.e. an anchor in another section or in a torn-down document.
        if (!node.isConnected || node.ownerDocument !== range.startContainer.ownerDocument)
            return false
        try {
            return range.comparePoint(node, anchor.startOffset) === 0
        } catch {
            // The anchored text node has been shortened since (IndexSizeError).
            return false
        }
    }
    // Determine which view is primary based on scroll position
    #detectPrimaryView() {
        if (this.#views.size <= 1) return
        const visibleStart = this.#renderedStart
        let offset = 0
        for (const [index, view] of this.#sortedViews) {
            const viewSize = view.element.getBoundingClientRect()[this.sideProp]
            if (visibleStart < offset + viewSize - 1) {
                if (index !== this.#primaryIndex) {
                    this.#primaryIndex = index
                    this.#syncA11y()
                    this.#trimDistantViews()
                    this.#replaceBackground()
                    this.#fillPromise = this.#preloadNext()
                }
                return
            }
            offset += viewSize
        }
    }
    // Pre-load adjacent sections from the current primary so the
    // next/prev sections are ready when the user paginates.
    // Does NOT re-scroll to avoid fighting with the user's current
    // scroll position.
    async #preloadNext() {
        if (this.noPreload || this.noContinuousScroll) return
        this.#filling = true
        try {
            const { size } = this
            const minPages = 5
            const maxSections = 8
            // Load forward sections until we have enough pages ahead
            let iterations = 0
            while (this.#views.size < maxSections && iterations < maxSections) {
                iterations++
                const pagesAhead = size > 0
                    ? Math.floor((this.#renderedViewSize - this.#renderedEnd) / size)
                    : 0
                if (pagesAhead >= minPages) break
                const sorted = this.#sortedViews
                const lastIndex = sorted[sorted.length - 1]?.[0]
                if (lastIndex == null) break
                const nextIdx = this.#adjacentIndex(1, lastIndex)
                if (nextIdx == null) break
                // Stop preloading at writing-mode boundaries
                if (!this.#isSameDirection(nextIdx)) break
                await this.#loadAdjacentSection(nextIdx)
                if (!this.#views.has(nextIdx)) break
            }
            // Wait a frame so ResizeObserver callbacks fire while
            // #filling is still true, preventing onExpand from
            // re-scrolling to a stale anchor position.
            await new Promise(r => requestAnimationFrame(r))
        } finally {
            this.#filling = false
            this.dispatchEvent(new Event('stabilized'))
        }
    }
    #afterScroll(reason) {
        // In multi-view, detect which section is primary
        if (this.#views.size > 1 && reason !== 'anchor' && reason !== 'navigation') {
            this.#detectPrimaryView()
            // Scrolling can bring a previously off-screen view into the
            // viewport (e.g. the next section's first column joining the
            // current section's last column in a dual-page spread) without
            // changing which view is primary. Re-sync a11y attributes so
            // a newly visible view stops being aria-hidden.
            this.#syncA11y()
        }
        const { range, index: visibleIndex } = this.#getVisibleRange() || {}
        if (!range) return
        this.#lastVisibleRange = range
        // don't set new anchor if relocation was to scroll to anchor
        if (reason === 'selection' || reason === 'navigation' || reason === 'anchor')
            this.#justAnchored = true
        // The scroll that re-anchoring itself performs (a resize re-render)
        // lands here through the debounced container scroll listener. Taking
        // the reflowed page's visible range as the new anchor then moves the
        // anchor to that page's start, which precedes the old anchor whenever
        // the two layouts' page boundaries differ, so the next resize back
        // settles one page earlier every time (readest#5808). A container
        // scroll that keeps the anchor on the page is not a navigation away
        // from it: keep the anchor.
        else if (reason !== 'container-scroll' || !this.#anchorIsVisible(range))
            this.#anchor = range

        const index = visibleIndex ?? this.#primaryIndex
        const primaryView = this.#primaryView
        const detail = { reason, range, index }
        if (this.scrolled) {
            // The relocated index may differ from #primaryIndex (the centre of
            // the viewport can sit in a different view than its top edge), so
            // size the fraction against the relocated view to keep it in sync.
            const indexView = this.#views.get(index) ?? primaryView
            const primaryOffset = this.#getViewOffset(index)
            const primarySize = indexView
                ? indexView.element.getBoundingClientRect()[this.sideProp] : this.#renderedViewSize
            detail.fraction = primarySize > 0
                ? Math.max(0, Math.min(1, (this.#renderedStart - primaryOffset) / primarySize)) : 0
        } else if (this.#renderedPages > 0 && primaryView) {
            const page = this.#renderedPage
            const pagesBeforePrimary = this.#getPagesBeforeView(index)
            const textPages = primaryView.contentPages
            this.#header.style.visibility = page > 0 ? 'visible' : 'hidden'
            // page is in spread units, textPages is in column units
            const localPage = page - pagesBeforePrimary
            const localColumn = localPage * this.columnCount
            detail.fraction = textPages > 0 ? Math.max(0, Math.min(1, localColumn / textPages)) : 0
            detail.size = textPages > 0 ? this.columnCount / textPages : 1
            if (reason === 'container-scroll' && localPage === 0) return
        }
        // Update per-column backgrounds for the current scroll position
        if (!this.scrolled) this.#replaceBackground()
        this.dispatchEvent(new CustomEvent('relocate', { detail }))
    }
    async #display(promise) {
        this.#stabilizing = true
        this.#container.style.opacity = '0'
        const { index, src, data, anchor, onLoad, select } = await promise
        this.#primaryIndex = index
        this.#syncA11y()
        const hasFocus = this.#primaryView?.document?.hasFocus()
        if (src) {
            const view = this.#createView(index)
            const afterLoad = doc => {
                if (doc.head) {
                    const $styleBefore = doc.createElement('style')
                    doc.head.prepend($styleBefore)
                    const $style = doc.createElement('style')
                    doc.head.append($style)
                    this.sections[index].spineProperties?.forEach(
                        prop => doc.documentElement.setAttribute('data-' + prop, ''))
                    this.#styleMap.set(doc, [$styleBefore, $style])
                }
                return onLoad?.({ doc, index })
            }
            const beforeRender = this.#beforeRender.bind(this)
            await view.load(src, data, afterLoad, beforeRender)
            if (this.#views.get(index) !== view) return
            if (!view.document?.documentElement || !view.document.body) {
                this.#destroyView(index)
                this.#primaryIndex = this.#sortedViews[0]?.[0] ?? -1
                this.#container.style.opacity = '1'
                this.#stabilizing = false
                this.dispatchEvent(new Event('stabilized'))
                return
            }
            // Cache direction for future preload boundary checks
            if (view.document) {
                const dir = getDirection(view.document)
                this.#directionCache.set(index, dir.vertical)
            }
            this.dispatchEvent(new CustomEvent('create-overlayer', {
                detail: {
                    doc: view.document, index,
                    attach: overlayer => view.overlayer = overlayer,
                },
            }))
        }
        // Pre-load previous section when needed:
        // - Short primary alignment (section shorter than one spread)
        // - Scrolled mode with anchor in top half — so the user can
        //   scroll backward into the previous section immediately
        const primaryView = this.#primaryView
        if (!this.noPreload && !this.noContinuousScroll && primaryView) {
            const needsPrev = (primaryView.contentPages > 0 && primaryView.contentPages < this.columnCount)
            if (needsPrev || this.scrolled) {
                const sorted = this.#sortedViews
                const firstIndex = sorted[0]?.[0]
                if (firstIndex != null) {
                    const prevIdx = this.#adjacentIndex(-1, firstIndex)
                    if (prevIdx != null && this.#isSameDirection(prevIdx)) {
                        await this.#loadAdjacentSection(prevIdx)
                    }
                }
            }
        }
        const resolvedAnchor = (typeof anchor === 'function'
            ? anchor(primaryView.document) : anchor) ?? 0
        await this.scrollToAnchor(resolvedAnchor, select)
        if (hasFocus) this.focusView()
        // Reveal content now that primary section is positioned
        this.#container.style.opacity = '1'
        this.#rendered = true
        // Emit stabilized so listeners can react, but keep #stabilizing
        // true until fill completes to prevent the debounced scroll
        // handler from loading backward sections during rapid DOM changes.
        this.dispatchEvent(new Event('stabilized'))
        // Load remaining adjacent sections progressively (non-blocking).
        // In scrolled mode, skip reanchor — browser scroll anchoring
        // preserves position when content is added above/below.
        this.#fillPromise = this.#fillVisibleArea(
            { reanchor: !this.scrolled })
        this.#fillPromise.then(() => { this.#stabilizing = false })
    }
    // Load an adjacent section without changing primary index
    async #loadAdjacentSection(index) {
        if (this.#views.has(index) || !this.#canGoToIndex(index)) return
        const section = this.sections[index]
        if (!section || section.linear === 'no') return
        // Detect a prepend: a section being inserted *above* every currently
        // loaded view in scrolled mode. The browser suppresses scroll
        // anchoring while scrollTop is 0, so the inserted section would push
        // the visible content down and the viewport would drift into the
        // previous section (readest/readest#4112). Capture the scroll position
        // before the insertion so it can be restored once the view renders.
        const firstIndex = this.#sortedViews[0]?.[0]
        const isPrepend = this.scrolled && firstIndex != null && index < firstIndex
        const startBefore = isPrepend ? this.#renderedStart : 0
        try {
            const src = await section.load()
            const data = await section.loadContent?.()
            const view = this.#createView(index)
            const afterLoad = async doc => {
                if (doc.head) {
                    const $styleBefore = doc.createElement('style')
                    doc.head.prepend($styleBefore)
                    const $style = doc.createElement('style')
                    doc.head.append($style)
                    section.spineProperties?.forEach(
                        prop => doc.documentElement.setAttribute('data-' + prop, ''))
                    this.#styleMap.set(doc, [$styleBefore, $style])
                }
                // A preload needs its own styles, not a restyle and font-ready
                // remeasurement of every chapter the reader can already see.
                this.#applyStyles(doc)
                await yieldToFrame()
                if (this.#views.get(index) !== view || view.document !== doc) return
                this.dispatchEvent(new CustomEvent('load', { detail: { doc, index } }))
            }
            // Adjacent sections reuse the primary view's cached layout
            // — they must NOT call #beforeRender, which would modify
            // global state (direction, CSS classes, dir attribute, etc.).
            const cachedLayout = this.#lastLayout
            const beforeRender = () => cachedLayout
            await view.load(src, data, afterLoad, beforeRender, async () => {
                // touchend queues the snap in rAF. Require two idle frames so
                // we cannot slip layout between finger release and that snap.
                // Stop waiting when navigation has discarded this view.
                for (let idleFrames = 0; idleFrames < 2 && this.#views.get(index) === view;) {
                    // Near the viewport this is required content, not spare
                    // buffer. Holding it back exposes an unrendered placeholder
                    // and can make boundary navigation skip the section.
                    if (this.#getViewOffset(index) <= this.#renderedEnd + this.size) return
                    await new Promise(resolve => requestAnimationFrame(resolve))
                    idleFrames = this.#touchState?.active || this.#isAnimating ? 0 : idleFrames + 1
                }
            })
            if (this.#views.get(index) !== view) return
            if (!view.document?.documentElement || !view.document.body) {
                this.#destroyView(index)
                return
            }
            // Cache direction for future preload boundary checks
            if (view.document) {
                const dir = getDirection(view.document)
                this.#directionCache.set(index, dir.vertical)
                // Destroy views with a different writing-mode immediately.
                // Mixed-direction views corrupt scroll/page calculations.
                if (dir.vertical !== this.#vertical) {
                    this.#destroyView(index)
                    return
                }
            }
            // Keep the previously visible content anchored: the new view added
            // `addedSize` px above it, so the scroll position must grow by the
            // same amount. This corrects the browser's scroll-anchoring
            // suppression at scrollTop 0 and is a no-op when anchoring already
            // handled the shift (correction ≈ 0).
            if (isPrepend) {
                const addedSize = view.element.getBoundingClientRect()[this.sideProp]
                const correction = startBefore + addedSize - this.#renderedStart
                if (Math.abs(correction) > 0.5)
                    this.containerPosition += (this.#vertical ? -1 : 1) * correction
            }
            this.dispatchEvent(new CustomEvent('create-overlayer', {
                detail: {
                    doc: view.document, index,
                    attach: overlayer => view.overlayer = overlayer,
                },
            }))
        } catch (e) {
            console.warn(e)
            console.warn(new Error(`Failed to load adjacent section ${index}`))
        }
    }
    // Fill adjacent sections until at least `minPages` pages exist
    // beyond the current viewport in each direction (forward always,
    // backward only when the primary section is short).
    // When reanchor is false (background pre-loading), skip re-scrolling
    // to avoid fighting with the user's current scroll position.
    async #fillVisibleArea({ reanchor = true } = {}) {
        if (this.noPreload || this.noContinuousScroll || this.#filling) return
        this.#filling = true
        try {
            const { size } = this
            if (!size) return
            const minPages = 5
            const maxSections = 8

            // If the primary section is shorter than one spread and
            // there's no section already loaded before it, load the
            // previous section to fill the leading columns
            const primaryView = this.#primaryView
            if (primaryView && primaryView.contentPages > 0
                && primaryView.contentPages < this.columnCount) {
                const sorted = this.#sortedViews
                const firstIndex = sorted[0]?.[0]
                if (firstIndex != null && firstIndex >= this.#primaryIndex) {
                    const prevIdx = this.#adjacentIndex(-1, firstIndex)
                    if (prevIdx != null && this.#isSameDirection(prevIdx)) {
                        await this.#loadAdjacentSection(prevIdx)
                    }
                }
            }

            // Load forward sections until we have enough pages ahead
            let iterations = 0
            while (this.#views.size < maxSections && iterations < maxSections) {
                iterations++
                const pagesAhead = Math.floor(
                    (this.#renderedViewSize - this.#renderedEnd) / size)
                if (pagesAhead >= minPages) break
                const sorted = this.#sortedViews
                const lastIndex = sorted[sorted.length - 1]?.[0]
                if (lastIndex == null) break
                const nextIdx = this.#adjacentIndex(1, lastIndex)
                if (nextIdx == null) break
                // Stop at writing-mode boundaries
                if (!this.#isSameDirection(nextIdx)) break
                await this.#loadAdjacentSection(nextIdx)
                if (!this.#views.has(nextIdx)) break
            }
            if (reanchor) this.#scrollToAnchor(this.#anchor)
        } finally {
            this.#filling = false
            // Emit stabilized so post-layout processing (e.g. warichu)
            // runs for newly loaded adjacent sections.
            this.dispatchEvent(new Event('stabilized'))
        }
    }
    // Trim views whose content is entirely more than 10 pages away
    // from the current viewport. Only removes views AFTER the primary
    // — removing views before would shift scroll position.
    #trimDistantViews() {
        const { size } = this
        if (!size) return
        const maxDistance = size * 10
        const viewportEnd = this.#renderedEnd
        for (const [index, view] of this.#sortedViews) {
            if (index <= this.#primaryIndex) continue
            const offset = this.#getViewOffset(index)
            if (offset - viewportEnd > maxDistance) {
                this.#destroyView(index)
            }
        }
    }
    #canGoToIndex(index) {
        return index >= 0 && index <= this.sections.length - 1
    }
    async #goTo({ index, anchor, select }) {
        const section = this.sections[index]
        if (!section) return
        // Check if the target section has a different writing-mode.
        // If direction changes, we must destroy all views and do a full
        // rebuild via #display — mixed-direction views cannot coexist.
        let directionChanged = false
        if (this.#views.has(index)) {
            const view = this.#views.get(index)
            if (view?.document) {
                const { vertical } = getDirection(view.document)
                directionChanged = vertical !== this.#vertical
            }
        } else if (this.#directionCache.has(index)) {
            directionChanged = this.#directionCache.get(index) !== this.#vertical
        }
        // When direction is unknown (not cached), #beforeRender will
        // detect and clean up stale views if a change actually occurs.

        if (this.#views.has(index) && !directionChanged) {
            // View already loaded — reuse it without
            // clearing/reloading. Just change primary and scroll.
            this.#stabilizing = true
            // Continuous scrolled mode keeps the target view rendered, so we
            // scroll straight to it without fading the container — fading
            // produced a hard blank-screen flash on adjacent navigation
            // (readest/readest#4112 follow-up). Paginated mode and discrete
            // no-continuous-scroll still fade to hide the page reposition.
            const blank = !this.scrolled || this.noContinuousScroll
            if (blank) this.#container.style.opacity = '0'
            const hasFocus = this.#primaryView?.document?.hasFocus()
            this.#primaryIndex = index
            this.#syncA11y()
            this.#trimDistantViews()
            // In noContinuousScroll mode, destroy all non-primary views
            if (this.noContinuousScroll) {
                for (const [i] of this.#views) {
                    if (i !== index) this.#destroyView(i)
                }
            }
            const primaryView = this.#primaryView
            const resolvedAnchor = (typeof anchor === 'function'
                ? anchor(primaryView.document) : anchor) ?? 0
            // Pre-load the previous section so the user can move backward right
            // away: a short paginated primary needs it to fill the leading
            // columns; scrolled mode needs it so scrolling up reveals the
            // previous section instead of dead-ending at the top (the debounced
            // backward-preload can't cover this — it bails while navigation is
            // stabilizing). Paginated must load it before revealing; scrolled
            // mode loads it after the scroll so the transition stays instant,
            // with #loadAdjacentSection compensation keeping the viewport
            // anchored as the section is inserted above.
            const needsPrev = primaryView && primaryView.contentPages > 0
                && primaryView.contentPages < this.columnCount
            const loadPrev = async () => {
                if (this.noPreload || this.noContinuousScroll) return
                if (!(needsPrev || this.scrolled)) return
                const firstIndex = this.#sortedViews[0]?.[0]
                if (firstIndex == null) return
                const prevIdx = this.#adjacentIndex(-1, firstIndex)
                if (prevIdx != null && this.#isSameDirection(prevIdx))
                    await this.#loadAdjacentSection(prevIdx)
            }
            if (!this.scrolled) await loadPrev()
            await this.scrollToAnchor(resolvedAnchor, select)
            if (this.scrolled) await loadPrev()
            if (blank) this.#container.style.opacity = '1'
            if (hasFocus) this.focusView()
            // Load remaining adjacent sections progressively;
            // keep #stabilizing true until fill completes
            this.#fillPromise = this.#fillVisibleArea()
            this.#fillPromise.then(() => { this.#stabilizing = false })
        } else {
            // When direction changes, clear ALL views — no reuse possible
            // across writing-mode boundaries. When direction is unknown
            // (not yet cached), keep nearby views; #beforeRender will
            // clean up if the loaded section turns out to differ.
            if (directionChanged) {
                this.#destroyAllViews()
            } else {
                const keep = new Set([index])
                if (!this.noContinuousScroll) {
                    for (const [i] of this.#views) {
                        if (Math.abs(i - index) <= 2) keep.add(i)
                    }
                }
                this.#clearViewsExcept(keep)
            }
            const oldIndex = this.#primaryIndex
            const onLoad = async detail => {
                if (oldIndex >= 0 && !this.#views.has(oldIndex))
                    this.sections[oldIndex]?.unload?.()
                this.#applyStyles(detail.doc)
                await yieldToFrame()
                if (this.#views.get(index)?.document !== detail.doc) return
                this.dispatchEvent(new CustomEvent('load', { detail }))
            }
            await this.#display(Promise.resolve(section.load())
                .then(async src => {
                    const data = await section.loadContent?.()
                    return { index, src, data, anchor, onLoad, select }
                }).catch(e => {
                    console.warn(e)
                    console.warn(new Error(`Failed to load section ${index}`))
                    return {}
                }))
        }
    }
    async goTo(target) {
        if (this.#locked) return
        const resolved = await target
        if (this.#canGoToIndex(resolved.index)) return this.#goTo(resolved)
    }
    #scrollPrev(distance) {
        if (this.#views.size === 0) return true
        if (this.scrolled) {
            if (this.#renderedStart > 0) return this.#scrollTo(
                Math.max(0, this.#renderedStart - (distance ?? this.size)), null, true)
            return !this.atStart
        }
        if (this.atStart) return
        const page = this.#renderedPage - 1
        // Out of range — skip animation, go straight to previous section
        if (page < 0) return true
        return this.#scrollToPage(page, 'page', true)
    }
    #scrollNext(distance) {
        if (this.#views.size === 0) return true
        if (this.scrolled) {
            if (this.#renderedViewSize - this.#renderedEnd > 2) return this.#scrollTo(
                Math.min(this.#renderedViewSize, distance ? this.#renderedStart + distance : this.#renderedEnd), null, true)
            return !this.atEnd
        }
        if (this.atEnd) return
        const page = this.#renderedPage + 1
        const pages = this.#renderedPages
        // Out of range — skip animation, go straight to next section
        if (page >= pages) return true
        return this.#scrollToPage(page, 'page', true)
    }
    get atStart() {
        const sorted = this.#sortedViews
        const firstIndex = sorted[0]?.[0] ?? this.#primaryIndex
        if (this.scrolled) return this.#adjacentIndex(-1, firstIndex) == null && this.#renderedStart <= 0
        return this.#adjacentIndex(-1, firstIndex) == null && this.#renderedPage <= 0
    }
    get atEnd() {
        const sorted = this.#sortedViews
        const lastIndex = sorted[sorted.length - 1]?.[0] ?? this.#primaryIndex
        if (this.scrolled) return this.#adjacentIndex(1, lastIndex) == null && this.#renderedViewSize - this.#renderedEnd <= 2
        return this.#adjacentIndex(1, lastIndex) == null && this.#renderedPage >= this.#renderedPages - 1
    }
    #adjacentIndex(dir, fromIndex) {
        if (fromIndex === undefined) fromIndex = this.#primaryIndex
        for (let index = fromIndex + dir; this.#canGoToIndex(index); index += dir)
            if (this.sections[index]?.linear !== 'no') return index
    }
    async #turnPage(dir, distance) {
        if (this.#locked) return
        this.#locked = true
        const prev = dir === -1
        const shouldGo = await (prev ? this.#scrollPrev(distance) : this.#scrollNext(distance))
        if (shouldGo) {
            // Wait for any in-progress background pre-loading to complete —
            // it may already be loading the section we need, so awaiting
            // it lets #goTo reuse the view instead of loading from scratch
            if (this.#fillPromise) await this.#fillPromise
            const sorted = this.#sortedViews
            const edgeIndex = prev
                ? sorted[0]?.[0] ?? this.#primaryIndex
                : sorted[sorted.length - 1]?.[0] ?? this.#primaryIndex
            await this.#goTo({
                index: this.#adjacentIndex(dir, edgeIndex),
                anchor: prev ? () => 1 : () => 0,
            })
        }
        if (shouldGo || !this.hasAttribute('animated')) await wait(100)
        this.#locked = false
    }
    async prev(distance) {
        return await this.#turnPage(-1, distance)
    }
    async next(distance) {
        return await this.#turnPage(1, distance)
    }
    async pan(dx, dy) {
        if (this.#locked) return
        this.#locked = true
        this.scrollBy(dx, dy)
        this.#locked = false
    }
    prevSection() {
        return this.goTo({ index: this.#adjacentIndex(-1) })
    }
    nextSection() {
        return this.goTo({ index: this.#adjacentIndex(1) })
    }
    firstSection() {
        const index = this.sections.findIndex(section => section.linear !== 'no')
        return this.goTo({ index })
    }
    lastSection() {
        const index = this.sections.findLastIndex(section => section.linear !== 'no')
        return this.goTo({ index })
    }
    getContents() {
        const contents = []
        for (const [index, view] of this.#sortedViews) {
            if (view.document) contents.push({
                index,
                overlayer: view.overlayer,
                doc: view.document,
            })
        }
        return contents
    }
    #applyStyles(doc) {
        const $$styles = this.#styleMap.get(doc)
        if (!$$styles) return
        const [$beforeStyle, $style] = $$styles
        const styles = this.#styles
        if (Array.isArray(styles)) {
            const [beforeStyle, style] = styles
            $beforeStyle.textContent = beforeStyle
            $style.textContent = style
        } else $style.textContent = styles
    }
    setStyles(styles) {
        this.#styles = styles
        for (const [, view] of this.#views) {
            if (!this.#styleMap.has(view.document)) continue
            this.#applyStyles(view.document)

            // needed because the resize observer doesn't work in Firefox
            fontsReady(view.document).then(() => view.expand())
        }

        // NOTE: needs `requestAnimationFrame` in Chromium
        const primaryView = this.#primaryView
        if (primaryView) {
            requestAnimationFrame(() => this.#replaceBackground())
        }
    }
    focusView() {
        this.#primaryView?.document?.defaultView?.focus()
    }
    showLoupe(winX, winY, { isVertical, color, gap, margin, radius, magnification }) {
        this.#primaryView?.showLoupe(winX, winY, { isVertical, color, gap, margin, radius, magnification })
    }
    hideLoupe() {
        this.#primaryView?.hideLoupe()
    }
    destroyLoupe() {
        this.#primaryView?.destroyLoupe()
    }
    destroy() {
        const transition = (this.#vtDrag ?? this.#vtFinishing)?.transition
            ?? this.#vtProgrammatic?.transition
        this.#vtDrag = null
        this.#vtFinishing = null
        this.#vtProgrammatic = null
        this.#slideTurnId++
        if (transition || this.#vtNamedHost) {
            transition?.ready?.catch(() => {})
            transition?.updateCallbackDone?.catch(() => {})
            try { transition?.skipTransition() } catch { /* already done */ }
            this.#vtCleanup()
            this.#isAnimating = false
        }
        this.#observer.unobserve(this)
        this.#destroyAllViews()
        this.#mediaQuery.removeEventListener('change', this.#mediaQueryListener)
    }
}

customElements.define('foliate-paginator', Paginator)
