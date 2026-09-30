// @ts-nocheck
import * as CFI from './epubcfi.js'

const NS = {
    CONTAINER: 'urn:oasis:names:tc:opendocument:xmlns:container',
    XHTML: 'http://www.w3.org/1999/xhtml',
    OPF: 'http://www.idpf.org/2007/opf',
    EPUB: 'http://www.idpf.org/2007/ops',
    DC: 'http://purl.org/dc/elements/1.1/',
    DCTERMS: 'http://purl.org/dc/terms/',
    ENC: 'http://www.w3.org/2001/04/xmlenc#',
    NCX: 'http://www.daisy.org/z3986/2005/ncx/',
    XLINK: 'http://www.w3.org/1999/xlink',
    SMIL: 'http://www.w3.org/ns/SMIL',
}

const MIME = {
    XML: 'application/xml',
    NCX: 'application/x-dtbncx+xml',
    XHTML: 'application/xhtml+xml',
    HTML: 'text/html',
    CSS: 'text/css',
    SVG: 'image/svg+xml',
    JS: /\/(x-)?(javascript|ecmascript)/,
}

// a document declared XHTML that the XML parser could not parse
const isBrokenXHTML = doc => doc.querySelector('parsererror')
    || !doc.documentElement?.namespaceURI
// Void elements that Adobe InDesign / Digital Editions exports leave unclosed
// (`<meta charset="utf-8">` constantly), the usual reason such a file is not
// well-formed XML. Lowercase only: XHTML is case-sensitive, and an uppercase
// `<BR>` closed as `<BR/>` would parse as an unknown element instead of a
// line break, so those files keep taking the HTML path.
const VOID_ELEMENT_RE = /<(meta|link|br|img|hr|input|col|area|base|embed|param|source|track|wbr)(?=[\s/>])((?:[^<>"']|"[^"]*"|'[^']*')*)>/g
const closeVoidElements = str => str.replace(VOID_ELEMENT_RE,
    (tag, name, attrs) => attrs.trimEnd().endsWith('/') ? tag : `<${name}${attrs}/>`)
// Parse a content document. A file the manifest declares as XHTML but which
// is not well-formed XML first gets its unclosed void elements closed and is
// parsed as XML again; only if that still fails is it parsed as HTML. The
// HTML parser is not a faithful reading of such a file: it ignores `/>` on
// non-void elements and re-opens formatting elements across blocks, so an
// `<a id="page_25"/>` at the top of a paragraph swallows every following
// `<p>` until the next anchor, and positions (CFIs, other readers' locators)
// stop matching the book's real structure. Shared by the render path
// (`loadReplaced`) and the off-screen path (`loadDocument`) so both see the
// same DOM.
const parseContentDocument = (parser, str, mediaType) => {
    let doc = parser.parseFromString(str, mediaType)
    if (mediaType !== MIME.XHTML || !isBrokenXHTML(doc)) return { doc, mediaType }
    const repaired = closeVoidElements(str)
    if (repaired !== str) {
        doc = parser.parseFromString(repaired, mediaType)
        if (!isBrokenXHTML(doc)) return { doc, mediaType }
    }
    console.warn(doc.querySelector('parsererror')?.innerText ?? 'Invalid XHTML')
    return { doc: parser.parseFromString(str, MIME.HTML), mediaType: MIME.HTML }
}

// https://www.w3.org/TR/epub-33/#sec-reserved-prefixes
const PREFIX = {
    a11y: 'http://www.idpf.org/epub/vocab/package/a11y/#',
    dcterms: 'http://purl.org/dc/terms/',
    marc: 'http://id.loc.gov/vocabulary/',
    media: 'http://www.idpf.org/epub/vocab/overlays/#',
    onix: 'http://www.editeur.org/ONIX/book/codelists/current.html#',
    rendition: 'http://www.idpf.org/vocab/rendition/#',
    schema: 'http://schema.org/',
    xsd: 'http://www.w3.org/2001/XMLSchema#',
    msv: 'http://www.idpf.org/epub/vocab/structure/magazine/#',
    prism: 'http://www.prismstandard.org/specifications/3.0/PRISM_CV_Spec_3.0.htm#',
}

const RELATORS = {
    art: 'artist',
    aut: 'author',
    clr: 'colorist',
    edt: 'editor',
    ill: 'illustrator',
    nrt: 'narrator',
    trl: 'translator',
    pbl: 'publisher',
}

const ONIX5 = {
    '02': 'isbn',
    '06': 'doi',
    '15': 'isbn',
    '26': 'doi',
    '34': 'issn',
}

// convert to camel case
const camel = x => x.toLowerCase().replace(/[-:](.)/g, (_, g) => g.toUpperCase())

// strip and collapse ASCII whitespace
// https://infra.spec.whatwg.org/#strip-and-collapse-ascii-whitespace
const normalizeWhitespace = str => str ? str
    .replace(/[\t\n\f\r ]+/g, ' ')
    .replace(/^[\t\n\f\r ]+/, '')
    .replace(/[\t\n\f\r ]+$/, '') : ''

const filterAttribute = (attr, value, isList) => isList
    ? el => el.getAttribute(attr)?.split(/\s/)?.includes(value)
    : typeof value === 'function'
        ? el => value(el.getAttribute(attr))
        : el => el.getAttribute(attr) === value

const getAttributes = (...xs) => el =>
    el ? Object.fromEntries(xs.map(x => [camel(x), el.getAttribute(x)])) : null

const getElementText = el => normalizeWhitespace(el?.textContent)

const childGetter = (doc, ns) => {
    // ignore the namespace if it doesn't appear in document at all
    const useNS = doc.lookupNamespaceURI(null) === ns || doc.lookupPrefix(ns)
    const f = useNS
        ? (el, name) => el => el.namespaceURI === ns && el.localName === name
        : (el, name) => el => el.localName === name
    return {
        $: (el, name) => [...el.children].find(f(el, name)),
        $$: (el, name) => [...el.children].filter(f(el, name)),
        $$$: useNS
            ? (el, name) => [...el.getElementsByTagNameNS(ns, name)]
            : (el, name) => [...el.getElementsByTagName(name)],
    }
}

// Zip entry names are raw, so a resolved href has to be fully decoded to match
// one. `decodeURI()` can't do it: by spec it preserves the reserved set
// (`; / ? : @ & = + $ , #`), leaving an entry named `a&b.html` unreachable
// behind its manifest href `a%26b.html`. Decode as a component instead, keeping
// only `/` and `#` encoded, which would otherwise turn into a path or fragment
// separator. Malformed escapes (a bare `%` in a name) decode to themselves.
const decodeURIPath = path => {
    try {
        return decodeURIComponent(path.replace(/%(2f|23)/gi, '%25$1'))
    } catch {
        return path
    }
}

const resolveURL = (url, relativeTo) => {
    try {
        if (isExternal(relativeTo)) return new URL(url, relativeTo)
        // the base needs to be a valid URL, so set a base URL and then remove it
        const root = 'https://invalid.invalid/'
        const obj = new URL(url, root + relativeTo)
        obj.search = ''
        return decodeURIPath(obj.href.replace(root, ''))
    } catch(e) {
        console.warn(e)
        return url
    }
}

const isExternal = uri => /^(?!blob)\w+:/i.test(uri)

// like `path.relative()` in Node.js
const pathRelative = (from, to) => {
    if (!from) return to
    const as = from.replace(/\/$/, '').split('/')
    const bs = to.replace(/\/$/, '').split('/')
    const i = (as.length > bs.length ? as : bs).findIndex((_, i) => as[i] !== bs[i])
    return i < 0 ? '' : Array(as.length - i).fill('..').concat(bs.slice(i)).join('/')
}

const pathDirname = str => str.slice(0, str.lastIndexOf('/') + 1)

// Run `f` over `items` with bounded concurrency, preserving order. Resource
// loads inside `loadReplaced` used to be fully serial -- one image/CSS after
// another, each paying its own ZIP read, which stacks up on image-heavy
// sections (worse on weak devices). Tasks are independent; a rejection still
// propagates through Promise.all, just like the old serial loop aborted at
// the first failure. Order is preserved by writing into preallocated slots.
const mapBounded = async (items, f, limit = 4) => {
    if (!items.length) return []
    const results = new Array(items.length)
    let next = 0
    await Promise.all(Array.from({ length: Math.min(limit, items.length) },
        async () => {
            while (next < items.length) {
                const i = next++
                results[i] = await f(items[i], i)
            }
        }))
    return results
}

// replace asynchronously; matches were resolved with bounded concurrency
// (was: strictly sequential) and are substituted back in match order
// same technique as https://stackoverflow.com/a/48032528
const replaceSeries = async (str, regex, f) => {
    const matches = []
    str.replace(regex, (...args) => (matches.push(args), null))
    const results = await mapBounded(matches, args => f(...args))
    return str.replace(regex, () => results.shift())
}

const regexEscape = str => str.replace(/[-/\\^$*+?.()|[\]{}]/g, '\\$&')

const tidy = obj => {
    for (const [key, val] of Object.entries(obj))
        if (val == null) delete obj[key]
        else if (Array.isArray(val)) {
            obj[key] = val.filter(x => x).map(x =>
                typeof x === 'object' && !Array.isArray(x) ? tidy(x) : x)
            if (!obj[key].length) delete obj[key]
            else if (obj[key].length === 1) obj[key] = obj[key][0]
        }
        else if (typeof val === 'object') {
            obj[key] = tidy(val)
            if (!Object.keys(val).length) delete obj[key]
        }
    const keys = Object.keys(obj)
    if (keys.length === 1 && keys[0] === 'name') return obj[keys[0]]
    return obj
}

// https://www.w3.org/TR/epub/#sec-prefix-attr
const getPrefixes = doc => {
    const map = new Map(Object.entries(PREFIX))
    const value = doc.documentElement.getAttributeNS(NS.EPUB, 'prefix')
        || doc.documentElement.getAttribute('prefix')
    if (value) for (const [, prefix, url] of value
        .matchAll(/(.+): +(.+)[ \t\r\n]*/g)) map.set(prefix, url)
    return map
}

// https://www.w3.org/TR/epub-rs/#sec-property-values
// but ignoring the case where the prefix is omitted
const getPropertyURL = (value, prefixes) => {
    if (!value) return null
    const [a, b] = value.split(':')
    const prefix = b ? a : null
    const reference = b ? b : a
    const baseURL = prefixes.get(prefix)
    return baseURL ? baseURL + reference : null
}

// See the call site in getMetadata() for the two calibre encodings this reads.
const getCalibreUserMetadata = (metaEls, legacyMeta) => {
    // calibre's to_json wraps non-JSON types; only datetime appears in columns
    const fromJSON = x => x?.__class__ === 'datetime.datetime' ? x.__value__ : x
    const isEmpty = (value, datatype) => value == null || value === ''
        || Array.isArray(value) && !value.length
        // calibre can't distinguish these from unset, and neither can we
        || datatype === 'datetime' && String(value).startsWith('0101-01-01')
        || datatype === 'rating' && !value
    const columns = []
    const add = (key, fm) => {
        if (!key?.startsWith('#') || typeof fm !== 'object' || !fm) return
        const datatype = fm.datatype ?? 'text'
        const value = fromJSON(fm['#value#'])
        if (isEmpty(value, datatype)) return
        const extra = fromJSON(fm['#extra#'])
        const label = key.slice(1)
        columns.push({
            label,
            name: typeof fm.name === 'string' && fm.name ? fm.name : label,
            datatype, value,
            ...extra != null ? { extra } : {},
        })
    }
    for (const el of metaEls ?? []) {
        if (el.getAttribute('property')?.toLowerCase() !== 'calibre:user_metadata') continue
        try {
            for (const [key, fm] of Object.entries(JSON.parse(getElementText(el))))
                add(key, fm)
        } catch {}
    }
    if (!columns.length)
        for (const [name, content] of Object.entries(legacyMeta ?? {})) {
            if (!name.startsWith('calibre:user_metadata:')) continue
            try {
                add(name.slice('calibre:user_metadata:'.length), JSON.parse(content))
            } catch {}
        }
    return columns.length ? columns : null
}

const getMetadata = opf => {
    const { $ } = childGetter(opf, NS.OPF)
    const $metadata = $(opf.documentElement, 'metadata')

    // first pass: convert to JS objects
    const els = Object.groupBy($metadata.children, el =>
        el.namespaceURI === NS.DC ? 'dc'
        : el.namespaceURI === NS.OPF && el.localName === 'meta' ?
            (el.hasAttribute('name') ? 'legacyMeta' : 'meta') : '')
    const baseLang = $metadata.getAttribute('xml:lang')
        ?? opf.documentElement.getAttribute('xml:lang') ?? 'und'
    const prefixes = getPrefixes(opf)
    const parse = el => {
        const property = el.getAttribute('property')
        const scheme = el.getAttribute('scheme')
        return {
            property: getPropertyURL(property, prefixes) ?? property,
            scheme: getPropertyURL(scheme, prefixes) ?? scheme,
            lang: el.getAttribute('xml:lang'),
            value: getElementText(el),
            props: getProperties(el),
            // `opf:` attributes from EPUB 2 & EPUB 3.1 (removed in EPUB 3.2)
            attrs: Object.fromEntries(Array.from(el.attributes)
                .filter(attr => attr.namespaceURI === NS.OPF)
                .map(attr => [attr.localName, attr.value])),
        }
    }
    const refines = Map.groupBy(els.meta ?? [], el => el.getAttribute('refines'))
    const getProperties = el => {
        const els = refines.get(el ? '#' + el.getAttribute('id') : null)
        if (!els) return null
        return Object.groupBy(els.map(parse), x => x.property)
    }
    const dc = Object.fromEntries(Object.entries(Object.groupBy(els.dc || [], el => el.localName))
        .map(([name, els]) => [name, els.map(parse)]))
    const properties = getProperties() ?? {}
    const legacyMeta = Object.fromEntries(els.legacyMeta?.map(el =>
        [el.getAttribute('name'), el.getAttribute('content')]) ?? [])

    // second pass: map to webpub
    const one = x => x?.[0]?.value
    const prop = (x, p) => one(x?.props?.[p])
    const makeLanguageMap = x => {
        if (!x) return null
        const alts = x.props?.['alternate-script'] ?? []
        const altRep = x.attrs['alt-rep']
        if (!alts.length && (!x.lang || x.lang === baseLang) && !altRep) return x.value
        const map = { [x.lang ?? baseLang]: x.value }
        if (altRep) map[x.attrs['alt-rep-lang']] = altRep
        for (const y of alts) map[y.lang] ??= y.value
        return map
    }
    const makeContributor = x => x ? ({
        name: makeLanguageMap(x),
        sortAs: makeLanguageMap(x.props?.['file-as']?.[0]) ?? x.attrs['file-as'],
        role: x.props?.role?.filter(x => x.scheme === PREFIX.marc + 'relators')
            ?.map(x => x.value) ?? [x.attrs.role],
        code: prop(x, 'term') ?? x.attrs.term,
        scheme: prop(x, 'authority') ?? x.attrs.authority,
    }) : null
    const makeCollection = x => ({
        name: makeLanguageMap(x),
        // NOTE: webpub requires number but EPUB allows values like "2.2.1"
        position: one(x.props?.['group-position']),
    })
    const makeSeries = x => ({
        name: x.value,
        position: one(x.props?.['group-position']),
    })
    const makeAltIdentifier = x => {
        const { value } = x
        if (/^urn:/i.test(value)) return value
        if (/^doi:/i.test(value)) return `urn:${value}`
        const type = x.props?.['identifier-type']
        if (!type) {
            const scheme = x.attrs.scheme
            if (!scheme) return value
            // https://idpf.github.io/epub-registries/identifiers/
            // but no "jdcn", which isn't a registered URN namespace
            if (/^(doi|isbn|uuid)$/i.test(scheme)) return `urn:${scheme}:${value}`
            // NOTE: webpub requires scheme to be a URI; EPUB allows anything
            return { scheme, value }
        }
        if (type.scheme === PREFIX.onix + 'codelist5') {
            const nid = ONIX5[type.value]
            if (nid) return `urn:${nid}:${value}`
        }
        return value
    }
    const belongsTo = Object.groupBy(properties['belongs-to-collection'] ?? [],
        x => prop(x, 'collection-type') === 'series' ? 'series' : 'collection')
    const mainTitle = dc.title?.find(x => prop(x, 'title-type') === 'main') ?? dc.title?.[0]
    const metadata = {
        identifier: getIdentifier(opf),
        title: makeLanguageMap(mainTitle),
        sortAs: makeLanguageMap(mainTitle?.props?.['file-as']?.[0])
            ?? mainTitle?.attrs?.['file-as']
            ?? legacyMeta?.['calibre:title_sort'],
        subtitle: dc.title?.find(x => prop(x, 'title-type') === 'subtitle')?.value,
        language: dc.language?.map(x => x.value),
        description: one(dc.description),
        publisher: makeContributor(dc.publisher?.[0]),
        published: dc.date?.find(x => x.attrs.event === 'publication')?.value
            ?? one(dc.date),
        modified: one(properties[PREFIX.dcterms + 'modified'])
            ?? dc.date?.find(x => x.attrs.event === 'modification')?.value,
        subject: dc.subject?.map(makeContributor),
        belongsTo: {
            collection: belongsTo.collection?.map(makeCollection),
            series: belongsTo.series?.map(makeSeries)
            ?? (legacyMeta?.['calibre:series'] ? {
                name: legacyMeta?.['calibre:series'],
                position: parseFloat(legacyMeta?.['calibre:series_index']),
            } : null),
        },
        altIdentifier: dc.identifier?.map(makeAltIdentifier),
        source: dc.source?.map(makeAltIdentifier), // NOTE: not in webpub schema
        rights: one(dc.rights), // NOTE: not in webpub schema
    }
    const remapContributor = defaultKey => x => {
        const keys = new Set(x.role?.map(role => RELATORS[role] ?? defaultKey))
        return [keys.size ? keys : [defaultKey], x]
    }
    for (const [keys, val] of [].concat(
        dc.creator?.map(makeContributor)?.map(remapContributor('author')) ?? [],
        dc.contributor?.map(makeContributor)?.map(remapContributor('contributor')) ?? []))
        for (const key of keys) {
            // if already parsed publisher don't remap it from author/contributor again
            if (key === 'publisher' && metadata.publisher) continue
            if (metadata[key]) metadata[key].push(val)
            else metadata[key] = [val]
        }
    tidy(metadata)
    if (metadata.altIdentifier === metadata.identifier)
        delete metadata.altIdentifier
    // Calibre embeds its custom columns ("user metadata") when polishing or
    // sending books. Two encodings (see calibre's opf2.py/opf3.py):
    //   OPF 2: <meta name="calibre:user_metadata:#label" content="{json}"/> per column
    //   OPF 3: a single <meta property="calibre:user_metadata"> whose text is
    //          a JSON dict of all columns keyed by "#label"; calibre prefers
    //          this form over the legacy metas when both are present
    // The column value lives in `#value#` (series index in `#extra#`);
    // datetimes are wrapped as {"__class__": "datetime.datetime",
    // "__value__": <ISO>} with 0101-01-01 meaning unset. Embedded files carry
    // every column of the library, so empty values are dropped here. Must run
    // after tidy(), which would otherwise collapse single-element value arrays.
    const calibreColumns = getCalibreUserMetadata(els.meta, legacyMeta)
    if (calibreColumns) metadata.calibreColumns = calibreColumns

    const rendition = {}
    const media = {}
    for (const [key, val] of Object.entries(properties)) {
        if (key.startsWith(PREFIX.rendition))
            rendition[camel(key.replace(PREFIX.rendition, ''))] = one(val)
        else if (key.startsWith(PREFIX.media))
            media[camel(key.replace(PREFIX.media, ''))] = one(val)
    }
    if (media.duration) media.duration = parseClock(media.duration)
    return { metadata, rendition, media }
}

const parseNav = (doc, resolve = f => f) => {
    const { $, $$, $$$ } = childGetter(doc, NS.XHTML)
    const resolveHref = href => href ? decodeURI(resolve(href)) : null
    const parseLI = getType => $li => {
        const $a = $($li, 'a') ?? $($li, 'span')
        const $ol = $($li, 'ol')
        const href = resolveHref($a?.getAttribute('href'))
        const label = getElementText($a) || $a?.getAttribute('title')
        // TODO: get and concat alt/title texts in content
        const result = { label, href, subitems: parseOL($ol) }
        if (getType) result.type = $a?.getAttributeNS(NS.EPUB, 'type')?.split(/\s/)
        return result
    }
    const parseOL = ($ol, getType) => $ol ? $$($ol, 'li').map(parseLI(getType)) : null
    const parseNav = ($nav, getType) => parseOL($($nav, 'ol'), getType)

    const $$nav = $$$(doc, 'nav')
    let toc = null, pageList = null, landmarks = null, others = []
    for (const $nav of $$nav) {
        const type = $nav.getAttributeNS(NS.EPUB, 'type')?.split(/\s/) ?? []
        if (type.includes('toc')) toc ??= parseNav($nav)
        else if (type.includes('page-list')) pageList ??= parseNav($nav)
        else if (type.includes('landmarks')) landmarks ??= parseNav($nav, true)
        else others.push({
            label: getElementText($nav.firstElementChild), type,
            list: parseNav($nav),
        })
    }
    return { toc, pageList, landmarks, others }
}

const parseNCX = (doc, resolve = f => f) => {
    const { $, $$ } = childGetter(doc, NS.NCX)
    const resolveHref = href => href ? decodeURI(resolve(href)) : null
    const parseItem = el => {
        const $label = $(el, 'navLabel')
        const $content = $(el, 'content')
        const label = getElementText($label)
        const href = resolveHref($content.getAttribute('src'))
        if (el.localName === 'navPoint') {
            const els = $$(el, 'navPoint')
            return { label, href, subitems: els.length ? els.map(parseItem) : null }
        }
        return { label, href }
    }
    const parseList = (el, itemName) => $$(el, itemName).map(parseItem)
    const getSingle = (container, itemName) => {
        const $container = $(doc.documentElement, container)
        return $container ? parseList($container, itemName) : null
    }
    return {
        toc: getSingle('navMap', 'navPoint'),
        pageList: getSingle('pageList', 'pageTarget'),
        others: $$(doc.documentElement, 'navList').map(el => ({
            label: getElementText($(el, 'navLabel')),
            list: parseList(el, 'navTarget'),
        })),
    }
}

const parseClock = str => {
    if (!str) return
    const parts = str.split(':').map(x => parseFloat(x))
    if (parts.length === 3) {
        const [h, m, s] = parts
        return h * 60 * 60 + m * 60 + s
    }
    if (parts.length === 2) {
        const [m, s] = parts
        return m * 60 + s
    }
    const [x, unit] = str.split(/(?=[^\d.])/)
    const n = parseFloat(x)
    const f = unit === 'h' ? 60 * 60
        : unit === 'min' ? 60
        : unit === 'ms' ? .001
        : 1
    return n * f
}

const IMAGE_EXTENSIONS = ['jpg', 'jpeg', 'png', 'gif', 'webp']
const FONT_EXTENSIONS = ['woff', 'woff2', 'ttf', 'otf']

const getImageMediaType = (path) => {
    const extension = path.toLowerCase().split('.').pop()
    const mediaTypeMap = {
        'jpg': 'image/jpeg',
        'jpeg': 'image/jpeg',
        'png': 'image/png',
        'gif': 'image/gif',
        'webp': 'image/webp',
        'svg': 'image/svg+xml',
    }
    return mediaTypeMap[extension] || 'image/jpeg'
}

const getFontMediaType = (path) => {
    const extension = path.toLowerCase().split('.').pop()
    const mediaTypeMap = {
        'woff': 'font/woff',
        'woff2': 'font/woff2',
        'ttf': 'font/ttf',
        'otf': 'font/otf',
    }
    return mediaTypeMap[extension] || 'font/ttf'
}

const AUDIO_VIDEO_MEDIA_TYPES = {
    'mp4': 'video/mp4',
    'm4v': 'video/mp4',
    'webm': 'video/webm',
    'ogv': 'video/ogg',
    'mov': 'video/quicktime',
    'mp3': 'audio/mpeg',
    'm4a': 'audio/mp4',
    'm4b': 'audio/mp4',
    'aac': 'audio/aac',
    'oga': 'audio/ogg',
    'ogg': 'audio/ogg',
    'opus': 'audio/ogg',
    'wav': 'audio/wav',
    'flac': 'audio/flac',
}

const AUDIO_VIDEO_EXTENSIONS = Object.keys(AUDIO_VIDEO_MEDIA_TYPES)

const getAudioVideoMediaType = path =>
    AUDIO_VIDEO_MEDIA_TYPES[path.toLowerCase().split('.').pop()]

// Container entry whose file name ends in `cover`/`couv` (the French
// spelling) plus an image extension, e.g. `cover.jpg`, `Images/Cover.PNG`,
// `couv.jpeg`. Same shape `gnome-epub-thumbnailer` falls back to.
const UNDECLARED_COVER_RE = /(?:cover|couv)\.(?:jpe?g|png|gif|webp|svg)$/i

// Last-ditch cover lookup for EPUBs where the manifest resolves to nothing:
// scan the container's own file names. `names` is iterated in central
// directory order, so the first match wins.
const findUndeclaredCover = names => {
    for (const name of names) if (UNDECLARED_COVER_RE.test(name)) return name
    return null
}

class MediaOverlay extends EventTarget {
    #entries
    #lastMediaOverlayItem
    #sectionIndex
    #audioIndex
    #itemIndex
    #audio
    #volume = 1
    #rate = 1
    #state
    constructor(book, loadXML) {
        super()
        this.book = book
        this.loadXML = loadXML
    }
    async #loadSMIL(item) {
        if (this.#lastMediaOverlayItem === item) return
        const doc = await this.loadXML(item.href)
        const resolve = href => href ? resolveURL(href, item.href) : null
        const { $, $$$ } = childGetter(doc, NS.SMIL)
        this.#audioIndex = -1
        this.#itemIndex = -1
        this.#entries = $$$(doc, 'par').reduce((arr, $par) => {
            const text = resolve($($par, 'text')?.getAttribute('src'))
            const $audio = $($par, 'audio')
            if (!text || !$audio) return arr
            const src = resolve($audio.getAttribute('src'))
            const begin = parseClock($audio.getAttribute('clipBegin'))
            const end = parseClock($audio.getAttribute('clipEnd'))
            const last = arr.at(-1)
            if (last?.src === src) last.items.push({ text, begin, end })
            else arr.push({ src, items: [{ text, begin, end }] })
            return arr
        }, [])
        this.#lastMediaOverlayItem = item
    }
    get #activeAudio() {
        return this.#entries[this.#audioIndex]
    }
    get #activeItem() {
        return this.#activeAudio?.items?.[this.#itemIndex]
    }
    #error(e) {
        console.error(e)
        this.dispatchEvent(new CustomEvent('error', { detail: e }))
    }
    #highlight() {
        this.dispatchEvent(new CustomEvent('highlight', { detail: this.#activeItem }))
    }
    #unhighlight() {
        this.dispatchEvent(new CustomEvent('unhighlight', { detail: this.#activeItem }))
    }
    async #play(audioIndex, itemIndex) {
        this.#stop()
        this.#audioIndex = audioIndex
        this.#itemIndex = itemIndex
        const src = this.#activeAudio?.src
        if (!src || !this.#activeItem) return this.start(this.#sectionIndex + 1)

        const url = URL.createObjectURL(await this.book.loadBlob(src))
        const audio = new Audio(url)
        this.#audio = audio
        audio.volume = this.#volume
        audio.playbackRate = this.#rate
        audio.addEventListener('timeupdate', () => {
            if (audio.paused) return
            const t = audio.currentTime
            const { items } = this.#activeAudio
            if (t > this.#activeItem?.end) {
                this.#unhighlight()
                if (this.#itemIndex === items.length - 1) {
                    this.#play(this.#audioIndex + 1, 0).catch(e => this.#error(e))
                    return
                }
            }
            const oldIndex = this.#itemIndex
            while (items[this.#itemIndex + 1]?.begin <= t) this.#itemIndex++
            if (this.#itemIndex !== oldIndex) this.#highlight()
        })
        audio.addEventListener('error', () =>
            this.#error(new Error(`Failed to load ${src}`)))
        audio.addEventListener('playing', () => this.#highlight())
        audio.addEventListener('ended', () => {
            this.#unhighlight()
            URL.revokeObjectURL(url)
            this.#audio = null
            this.#play(audioIndex + 1, 0).catch(e => this.#error(e))
        })
        if (this.#state === 'paused') {
            this.#highlight()
            audio.currentTime = this.#activeItem.begin ?? 0
        }
        else audio.addEventListener('canplaythrough', () => {
            // for some reason need to seek in `canplaythrough`
            // or it won't play when skipping in WebKit
            audio.currentTime = this.#activeItem.begin ?? 0
            this.#state = 'playing'
            audio.play().catch(e => this.#error(e))
        }, { once: true })
    }
    async start(sectionIndex, filter = () => true) {
        this.#audio?.pause()
        const section = this.book.sections[sectionIndex]
        const href = section?.id
        if (!href) return

        const { mediaOverlay } = section
        if (!mediaOverlay) return this.start(sectionIndex + 1)
        this.#sectionIndex = sectionIndex
        await this.#loadSMIL(mediaOverlay)

        for (let i = 0; i < this.#entries.length; i++) {
            const { items } = this.#entries[i]
            for (let j = 0; j < items.length; j++) {
                if (items[j].text.split('#')[0] === href && filter(items[j], j, items))
                    return this.#play(i, j).catch(e => this.#error(e))
            }
        }
    }
    pause() {
        this.#state = 'paused'
        this.#audio?.pause()
    }
    resume() {
        this.#state = 'playing'
        this.#audio?.play().catch(e => this.#error(e))
    }
    #stop() {
        if (this.#audio) {
            this.#audio.pause()
            URL.revokeObjectURL(this.#audio.src)
            this.#audio = null
            this.#unhighlight()
        }
    }
    stop() {
        this.#state = 'stopped'
        this.#stop()
    }
    prev() {
        if (this.#itemIndex > 0) this.#play(this.#audioIndex, this.#itemIndex - 1)
        else if (this.#audioIndex > 0) this.#play(this.#audioIndex - 1,
            this.#entries[this.#audioIndex - 1].items.length - 1)
        else if (this.#sectionIndex > 0)
            this.start(this.#sectionIndex - 1, (_, i, items) => i === items.length - 1)
    }
    next() {
        this.#play(this.#audioIndex, this.#itemIndex + 1)
    }
    setVolume(volume) {
        this.#volume = volume
        if (this.#audio) this.#audio.volume = volume
    }
    setRate(rate) {
        this.#rate = rate
        if (this.#audio) this.#audio.playbackRate = rate
    }
}

const isUUID = /([0-9a-f]{8})-([0-9a-f]{4})-([0-9a-f]{4})-([0-9a-f]{4})-([0-9a-f]{12})/i

const getUUID = opf => {
    const extractUUID = el => {
        const text = getElementText(el)
        const id = text.split(':').slice(-1)[0]
        const match = isUUID.exec(id)
        return match ? match[0] : null
    }
    const identifiers = Array.from(opf.getElementsByTagNameNS(NS.DC, 'identifier'))
    // 1. Prefer the unique-identifier (used by Adobe font obfuscation)
    const uniqueIdAttr = opf.documentElement.getAttribute('unique-identifier')
    if (uniqueIdAttr) {
        const el = identifiers.find(el => el.getAttribute('id') === uniqueIdAttr)
        if (el) {
            const uuid = extractUUID(el)
            if (uuid) return uuid
        }
    }
    // 2. Prefer urn:uuid: identifiers (standard UUID URN per RFC 4122)
    for (const el of identifiers) {
        const text = getElementText(el)
        if (/^urn:uuid:/i.test(text)) {
            const uuid = extractUUID(el)
            if (uuid) return uuid
        }
    }
    // 3. Fall back to any identifier containing a UUID
    for (const el of identifiers) {
        const uuid = extractUUID(el)
        if (uuid) return uuid
    }
    return ''
}

const getIdentifier = opf => getElementText(
    opf.getElementById(opf.documentElement.getAttribute('unique-identifier'))
    ?? opf.getElementsByTagNameNS(NS.DC, 'identifier')[0])

// https://www.w3.org/publishing/epub32/epub-ocf.html#sec-resource-obfuscation
const deobfuscate = async (key, length, blob) => {
    const array = new Uint8Array(await blob.slice(0, length).arrayBuffer())
    length = Math.min(length, array.length)
    for (var i = 0; i < length; i++) array[i] = array[i] ^ key[i % key.length]
    return new Blob([array, blob.slice(length)], { type: blob.type })
}

const WebCryptoSHA1 = async str => {
    const data = new TextEncoder().encode(str)
    const buffer = await globalThis.crypto.subtle.digest('SHA-1', data)
    return new Uint8Array(buffer)
}

const deobfuscators = (sha1 = WebCryptoSHA1) => ({
    'http://www.idpf.org/2008/embedding': {
        key: opf => sha1(getIdentifier(opf)
            // eslint-disable-next-line no-control-regex
            .replaceAll(/[\u0020\u0009\u000d\u000a]/g, '')),
        decode: (key, blob) => deobfuscate(key, 1040, blob),
    },
    'http://ns.adobe.com/pdf/enc#RC': {
        key: opf => {
            const uuid = getUUID(opf).replaceAll('-', '')
            return Uint8Array.from({ length: 16 }, (_, i) =>
                parseInt(uuid.slice(i * 2, i * 2 + 2), 16))
        },
        decode: (key, blob) => deobfuscate(key, 1024, blob),
    },
})

class Encryption {
    #uris = new Map()
    #decoders = new Map()
    #algorithms
    constructor(algorithms) {
        this.#algorithms = algorithms
    }
    async init(encryption, opf) {
        if (!encryption) return
        const data = Array.from(
            encryption.getElementsByTagNameNS(NS.ENC, 'EncryptedData'), el => ({
                algorithm: el.getElementsByTagNameNS(NS.ENC, 'EncryptionMethod')[0]
                    ?.getAttribute('Algorithm'),
                uri: el.getElementsByTagNameNS(NS.ENC, 'CipherReference')[0]
                    ?.getAttribute('URI'),
            }))
        for (const { algorithm, uri } of data) {
            if (!this.#decoders.has(algorithm)) {
                const algo = this.#algorithms[algorithm]
                if (!algo) {
                    console.warn('Unknown encryption algorithm')
                    continue
                }
                const key = await algo.key(opf)
                this.#decoders.set(algorithm, blob => algo.decode(key, blob))
            }
            this.#uris.set(uri, algorithm)
        }
    }
    getDecoder(uri) {
        return this.#decoders.get(this.#uris.get(uri)) ?? (x => x)
    }
}

class Resources {
    constructor({ opf, resolveHref }) {
        this.opf = opf
        const { $, $$, $$$ } = childGetter(opf, NS.OPF)

        const $manifest = $(opf.documentElement, 'manifest')
        const $spine = $(opf.documentElement, 'spine')
        const $$itemref = $$($spine, 'itemref')

        this.manifest = $$($manifest, 'item')
            .map(getAttributes('href', 'id', 'media-type', 'properties', 'media-overlay'))
            .map(item => {
                item.href = resolveHref(item.href)
                item.properties = item.properties?.split(/\s/)
                return item
            })
        this.manifestById = new Map(this.manifest.map(item => [item.id, item]))
        this.spine = $$itemref
            .map(getAttributes('idref', 'id', 'linear', 'properties'))
            .map(item => (item.properties = item.properties?.split(/\s/), item))
        this.pageProgressionDirection = $spine
            .getAttribute('page-progression-direction')

        this.navPath = this.getItemByProperty('nav')?.href
        this.ncxPath = (this.getItemByID($spine.getAttribute('toc'))
            ?? this.manifest.find(item => item.mediaType === MIME.NCX))?.href

        const $guide = $(opf.documentElement, 'guide')
        if ($guide) this.guide = $$($guide, 'reference')
            .map(getAttributes('type', 'title', 'href'))
            .map(({ type, title, href }) => ({
                label: title,
                type: type.split(/\s/),
                href: resolveHref(href),
            }))

        this.cover = this.getItemByProperty('cover-image')
            // EPUB 2 compat
            ?? this.getItemByID($$$(opf, 'meta')
                .find(filterAttribute('name', 'cover'))
                ?.getAttribute('content'))
            ?? this.manifest.find(item => item.id === 'cover'
                && item.mediaType.startsWith('image'))
            ?? this.manifest.find(item => item.href.includes('cover')
                && item.mediaType.startsWith('image'))
            ?? this.getItemByHref(this.guide
                ?.find(ref => ref.type.includes('cover'))?.href)
            // last resort: first image in manifest
            ?? this.manifest.find(item => item.mediaType.startsWith('image'))

        this.cfis = CFI.fromElements($$itemref)
    }
    getItemByID(id) {
        return this.manifestById.get(id)
    }
    getItemByHref(href) {
        return this.manifest.find(item => item.href === href)
    }
    getItemByProperty(prop) {
        return this.manifest.find(item => item.properties?.includes(prop))
    }
    resolveCFI(cfi) {
        const parts = CFI.parse(cfi)
        const top = (parts.parent ?? parts).shift()
        let $itemref = CFI.toElement(this.opf, top)
        // make sure it's an idref; if not, try again without the ID assertion
        // mainly because Epub.js used to generate wrong ID assertions
        // https://github.com/futurepress/epub.js/issues/1236
        if ($itemref && $itemref.nodeName !== 'idref') {
            top.at(-1).id = null
            $itemref = CFI.toElement(this.opf, top)
        }
        const idref = $itemref?.getAttribute('idref')
        const index = this.spine.findIndex(item => item.idref === idref)
        const anchor = doc => CFI.toRange(doc, parts)
        return { index, anchor }
    }
}

class Loader {
    #cache = new Map()
    #cacheXHTMLContent = new Map()
    #children = new Map()
    #refCount = new Map()
    eventTarget = new EventTarget()
    constructor({ loadText, loadBlob, resources, entries }) {
        this.loadText = loadText
        this.loadBlob = loadBlob
        this.manifest = resources.manifest
        this.assets = resources.manifest
        this.entries = entries
        // needed only when replacing in (X)HTML w/o parsing (see below)
        //.filter(({ mediaType }) => ![MIME.XHTML, MIME.HTML].includes(mediaType))
    }
    async createURL(href, data, type, parent) {
        if (!data) return ''
        const detail = { data, type }
        Object.defineProperty(detail, 'name', { value: href }) // readonly
        const event = new CustomEvent('data', { detail })
        this.eventTarget.dispatchEvent(event)
        const newData = await event.detail.data
        const newType = await event.detail.type
        const url = URL.createObjectURL(new Blob([newData], { type: newType }))
        this.#cache.set(href, url)
        this.#refCount.set(href, 1)
        if (newType === MIME.XHTML || newType === MIME.HTML) {
            this.#cacheXHTMLContent.set(url, {href, type: newType, data: newData})
        }
        if (parent) {
            const childList = this.#children.get(parent)
            if (childList) childList.push(href)
            else this.#children.set(parent, [href])
        }
        return url
    }
    ref(href, parent) {
        // A top-level load -- a view opening a section -- has no parent
        // document to hang the reference on, and is released by exactly one
        // `unloadItem`, so it must always be counted. Recording it under an
        // absent parent instead put every top-level load in the book into one
        // shared `#children` bucket that nothing ever cleared, so the second
        // view to open an already-loaded section (a footnote popup, which
        // opens another view on the same book) skipped its increment yet still
        // decremented on close. The count underflowed to zero and revoked the
        // section along with its images while a view was still showing them.
        if (!parent) {
            this.#refCount.set(href, this.#refCount.get(href) + 1)
            return this.#cache.get(href)
        }
        const childList = this.#children.get(parent)
        if (!childList?.includes(href)) {
            this.#refCount.set(href, this.#refCount.get(href) + 1)
            //console.log(`referencing ${href}, now ${this.#refCount.get(href)}`)
            if (childList) childList.push(href)
            else this.#children.set(parent, [href])
        }
        return this.#cache.get(href)
    }
    unref(href) {
        if (!this.#refCount.has(href)) return
        const count = this.#refCount.get(href) - 1
        //console.log(`unreferencing ${href}, now ${count}`)
        if (count < 1) {
            //console.log(`unloading ${href}`)
            const url = this.#cache.get(href)
            URL.revokeObjectURL(url)
            this.#cache.delete(href)
            this.#cacheXHTMLContent.delete(url)
            this.#refCount.delete(href)
            // unref children
            const childList = this.#children.get(href)
            if (childList) while (childList.length) this.unref(childList.pop())
            this.#children.delete(href)
        } else this.#refCount.set(href, count)
    }
    // load manifest item, recursively loading all resources as needed
    async loadItem(item, parents = []) {
        if (!item) return null
        const { href, mediaType } = item

        const isScript = MIME.JS.test(item.mediaType)
        const detail = { type: mediaType, href, isScript, allow: true}
        const event = new CustomEvent('load', { detail })
        this.eventTarget.dispatchEvent(event)
        const { allow, url } = await event.detail
        if (!allow) return null
        if (url !== undefined) return url

        const parent = parents.at(-1)
        if (this.#cache.has(href)) return this.ref(href, parent)

        const shouldReplace =
            (isScript || [MIME.XHTML, MIME.HTML, MIME.CSS, MIME.SVG].includes(mediaType))
            // prevent circular references
            && parents.every(p => p !== href)
        if (shouldReplace) return this.loadReplaced(item, parents)
        // NOTE: this can be replaced with `Promise.try()`
        const tryLoadBlob = Promise.resolve().then(() => this.loadBlob(href))
        return this.createURL(href, tryLoadBlob, mediaType, parent)
    }
    async loadItemXHTMLContent(item, parents = []) {
        // Callers read the source of a section they have just loaded (the
        // renderer pairs `section.load()` with `section.loadContent()`), and
        // there is no matching unload for this call, so reuse the reference
        // they already hold rather than taking one that is never released.
        const url = this.#cache.get(item?.href) ?? await this.loadItem(item, parents)
        if (url) return this.#cacheXHTMLContent.get(url)?.data
    }
    tryImageEntryItem(path) {
        if (!IMAGE_EXTENSIONS.some(ext => path.toLowerCase().endsWith(`.${ext}`))) {
            return null
        }
        if (!this.entries.get(path)) {
            return null
        }
        return {
            href: path,
            mediaType: getImageMediaType(path),
        }
    }
    tryFontEntryItem(path) {
        if (!FONT_EXTENSIONS.some(ext => path.toLowerCase().endsWith(`.${ext}`))) {
            return null
        }
        if (this.entries.get(path)) {
            return {
                href: path,
                mediaType: getFontMediaType(path),
            }
        }
        return {
            href: `fonts/${path.split('/').pop()}`,
            mediaType: getFontMediaType(path),
        }
    }
    // Builders like epubBuilder (cnepub) ship audio and video in the container
    // but never declare them in the manifest, the same way they leave out the
    // illustrations. A relative href can't resolve against the section's
    // `blob:` URL, so the media element would otherwise end up with no source.
    tryAudioVideoEntryItem(path) {
        if (!AUDIO_VIDEO_EXTENSIONS.some(ext => path.toLowerCase().endsWith(`.${ext}`))) {
            return null
        }
        if (!this.entries.get(path)) {
            return null
        }
        return {
            href: path,
            mediaType: getAudioVideoMediaType(path),
        }
    }
    async loadHref(href, base, parents = []) {
        if (isExternal(href)) return href
        const path = resolveURL(href, base)
        let item = this.manifest.find(item => item.href === path)
        if (!item) {
            item = this.tryImageEntryItem(path) ?? this.tryFontEntryItem(path)
                ?? this.tryAudioVideoEntryItem(path)
            if (!item) {
                return href
            }
        }
        return this.loadItem(item, parents.concat(base))
    }
    async loadReplaced(item, parents = []) {
        const { href, mediaType } = item
        const parent = parents.at(-1)
        let str = ''
        try {
            str = await this.loadText(href)
        } catch (e) {
            return this.createURL(href, Promise.reject(e), mediaType, parent)
        }
        if (!str) return null

        // note that one can also just use `replaceString` for everything:
        // ```
        // const replaced = await this.replaceString(str, href, parents)
        // return this.createURL(href, replaced, mediaType, parent)
        // ```
        // which is basically what Epub.js does, which is simpler, but will
        // break things like iframes (because you don't want to replace links)
        // or text that just happen to be paths

        // parse and replace in HTML
        if ([MIME.XHTML, MIME.HTML, MIME.SVG].includes(mediaType)) {
            const parsed = parseContentDocument(new DOMParser(), str, mediaType)
            const doc = parsed.doc
            // it's now HTML if it wasn't valid XHTML even after repair
            item.mediaType = parsed.mediaType
            // replace hrefs in XML processing instructions
            // this is mainly for SVGs that use xml-stylesheet
            if ([MIME.XHTML, MIME.SVG].includes(item.mediaType)) {
                let child = doc.firstChild
                while (child instanceof ProcessingInstruction) {
                    if (child.data) {
                        const replacedData = await replaceSeries(child.data,
                            /(?:^|\s*)(href\s*=\s*['"])([^'"]*)(['"])/i,
                            (_, p1, p2, p3) => this.loadHref(p2, href, parents)
                                .then(p2 => `${p1}${p2}${p3}`))
                        child.replaceWith(doc.createProcessingInstruction(
                            child.target, replacedData))
                    }
                    child = child.nextSibling
                }
            }
            // replace hrefs (excluding anchors)
            const replace = async (el, attr) => el.setAttribute(attr,
                await this.loadHref(el.getAttribute(attr), href, parents))
            const attrTasks = [
                [...doc.querySelectorAll('link[href]')].map(el => [el, 'href']),
                [...doc.querySelectorAll('[src]')].map(el => [el, 'src']),
                [...doc.querySelectorAll('[poster]')].map(el => [el, 'poster']),
                [...doc.querySelectorAll('object[data]')].map(el => [el, 'data']),
            ].flat()
            await mapBounded(attrTasks, ([el, attr]) => replace(el, attr))
            await mapBounded([...doc.querySelectorAll('[*|href]:not([href])')],
                async el => el.setAttributeNS(NS.XLINK, 'href', await this.loadHref(
                    el.getAttributeNS(NS.XLINK, 'href'), href, parents)))
            await mapBounded([...doc.querySelectorAll('[srcset]')],
                async el => el.setAttribute('srcset', await replaceSeries(el.getAttribute('srcset'),
                    /(\s*)(.+?)\s*((?:\s[\d.]+[wx])+\s*(?:,|$)|,\s+|$)/g,
                    (_, p1, p2, p3) => this.loadHref(p2, href, parents)
                        .then(p2 => `${p1}${p2}${p3}`))))
            // replace inline styles
            await mapBounded([...doc.querySelectorAll('style')], async el => {
                if (el.textContent) el.textContent =
                    await this.replaceCSS(el.textContent, href, parents)
            })
            await mapBounded([...doc.querySelectorAll('[style]')],
                async el => el.setAttribute('style',
                    await this.replaceCSS(el.getAttribute('style'), href, parents)))
            // TODO: replace inline scripts? probably not worth the trouble
            const result = new XMLSerializer().serializeToString(doc)
            return this.createURL(href, result, item.mediaType, parent)
        }

        const result = mediaType === MIME.CSS
            ? await this.replaceCSS(str, href, parents)
            : await this.replaceString(str, href, parents)
        return this.createURL(href, result, mediaType, parent)
    }
    async replaceCSS(str, href, parents = []) {
        const replacedUrls = await replaceSeries(str,
            /url\(\s*["']?([^'"\n]*?)\s*["']?\s*\)/gi,
            (_, url) => this.loadHref(url, href, parents)
                .then(url => `url("${url}")`))
        // apart from `url()`, strings can be used for `@import` (but why?!)
        return replaceSeries(replacedUrls,
            /@import\s*["']([^"'\n]*?)["']/gi,
            (_, url) => this.loadHref(url, href, parents)
                .then(url => `@import "${url}"`))
    }
    // find & replace all possible relative paths for all assets without parsing
    replaceString(str, href, parents = []) {
        const assetMap = new Map()
        const urls = this.assets.map(asset => {
            // do not replace references to the file itself
            if (asset.href === href) return
            // href was decoded and resolved when parsing the manifest
            const relative = pathRelative(pathDirname(href), asset.href)
            const relativeEnc = encodeURI(relative)
            const rootRelative = '/' + asset.href
            const rootRelativeEnc = encodeURI(rootRelative)
            const set = new Set([relative, relativeEnc, rootRelative, rootRelativeEnc])
            for (const url of set) assetMap.set(url, asset)
            return Array.from(set)
        }).flat().filter(x => x)
        if (!urls.length) return str
        const regex = new RegExp(urls.map(regexEscape).join('|'), 'g')
        return replaceSeries(str, regex, async match =>
            this.loadItem(assetMap.get(match.replace(/^\//, '')),
                parents.concat(href)))
    }
    unloadItem(item) {
        this.unref(item?.href)
    }
    destroy() {
        for (const url of this.#cache.values()) URL.revokeObjectURL(url)
    }
}

const getHTMLFragment = (doc, id) => doc.getElementById(id)
    ?? doc.querySelector(`[name="${CSS.escape(id)}"]`)

const getPageSpread = properties => {
    for (const p of properties) {
        if (p === 'page-spread-left' || p === 'rendition:page-spread-left')
            return 'left'
        if (p === 'page-spread-right' || p === 'rendition:page-spread-right')
            return 'right'
        if (p === 'rendition:page-spread-center') return 'center'
    }
}

const getDisplayOptions = doc => {
    if (!doc) return null
    return {
        fixedLayout: getElementText(doc.querySelector('option[name="fixed-layout"]')),
        openToSpread: getElementText(doc.querySelector('option[name="open-to-spread"]')),
    }
}

// Some EPUBs ship an OPF/NCX/nav doc that isn't well-formed XML: either named
// HTML entities that XML doesn't predefine (`&nbsp;` …), or — worse — a bare
// `&` that was never escaped (e.g. a hand-built manifest id like
// `id="Search_&_Rescue"`). A strict XML parser rejects both, failing the whole
// import. Map the known named entities to numeric refs, then escape any
// remaining `&` that doesn't begin a valid character/entity reference so the
// document parses instead.
const xmlNamedEntities = {
    nbsp: '&#160;', mdash: '&#8212;', ndash: '&#8211;',
    ldquo: '&#8220;', rdquo: '&#8221;', lsquo: '&#8216;', rsquo: '&#8217;',
    hellip: '&#8230;', copy: '&#169;', reg: '&#174;', trade: '&#8482;',
    bull: '&#8226;', middot: '&#183;',
}
const sanitizeXMLEntities = str => str
    .replace(/&([a-z]+);/gi, (match, entity) =>
        xmlNamedEntities[entity.toLowerCase()] ?? match)
    .replace(/&(?!#\d+;|#x[0-9a-f]+;|[a-z][a-z0-9]*;)/gi, '&amp;')

export class EPUB {
    parser = new DOMParser()
    #loader
    #encryption
    constructor({ entries, loadText, loadBlob, getSize, sha1 }) {
        this.entries = entries.reduce((map, entry) => {
            map.set(entry.filename, entry)
            return map
        }, new Map())
        this.loadText = loadText
        this.loadBlob = loadBlob
        this.getSize = getSize
        this.#encryption = new Encryption(deobfuscators(sha1))
    }
    async #loadXML(uri) {
        const str = await this.loadText(uri)
        if (!str) return null
        const sanitized = sanitizeXMLEntities(str)
        const doc = this.parser.parseFromString(sanitized, MIME.XML)
        if (doc.querySelector('parsererror'))
            throw new Error(`XML parsing error: ${uri}
${doc.querySelector('parsererror').innerText}`)
        return doc
    }
    async init() {
        const $container = await this.#loadXML('META-INF/container.xml')
        if (!$container) throw new Error('Failed to load container file')

        const opfs = Array.from(
            $container.getElementsByTagNameNS(NS.CONTAINER, 'rootfile'),
            getAttributes('full-path', 'media-type'))
            .filter(file => file.mediaType === 'application/oebps-package+xml')

        if (!opfs.length) throw new Error('No package document defined in container')
        const opfPath = opfs[0].fullPath
        const opf = await this.#loadXML(opfPath)
        if (!opf) throw new Error('Failed to load package document')

        const $encryption = await this.#loadXML('META-INF/encryption.xml')
        await this.#encryption.init($encryption, opf)

        this.resources = new Resources({
            opf,
            resolveHref: url => resolveURL(url, opfPath),
        })
        this.#loader = new Loader({
            loadText: this.loadText,
            loadBlob: uri => Promise.resolve(this.loadBlob(uri))
                .then(this.#encryption.getDecoder(uri)),
            resources: this.resources,
            entries: this.entries,
        })
        this.transformTarget = this.#loader.eventTarget
        this.sections = this.resources.spine.map((spineItem, index) => {
            const { idref, linear, properties = [] } = spineItem
            const item = this.resources.getItemByID(idref)
            if (!item) {
                console.warn(`Could not find item with ID "${idref}" in manifest`)
                return null
            }
            return {
                id: item.href,
                load: () => this.#loader.loadItem(item),
                unload: () => this.#loader.unloadItem(item),
                loadText: () => this.#loader.loadText(item.href),
                loadContent: () => this.#loader.loadItemXHTMLContent(item),
                // Load a resource a script references after the section was
                // rendered (a <video src> built on click); `loadReplaced` only
                // saw what was in the markup. The section is its parent, so it
                // is released together with the section.
                loadHref: href => this.#loader.loadHref(href, item.href),
                createDocument: () => this.loadDocument(item),
                size: this.getSize(item.href),
                cfi: this.resources.cfis[index],
                linear,
                spineProperties: properties,
                pageSpread: getPageSpread(properties),
                resolveHref: href => resolveURL(href, item.href),
                mediaOverlay: item.mediaOverlay
                    ? this.resources.getItemByID(item.mediaOverlay) : null,
            }
        }).filter(s => s)

        const { navPath, ncxPath } = this.resources
        if (navPath) try {
            const resolve = url => resolveURL(url, navPath)
            const nav = parseNav(await this.#loadXML(navPath), resolve)
            this.toc = nav.toc
            this.pageList = nav.pageList
            this.landmarks = nav.landmarks
        } catch(e) {
            console.warn(e)
        }
        // Some publishers ship an EPUB3 nav doc whose <li>s contain only
        // plain text (no <a href>). parseNav returns a non-empty array, so
        // the original check `if (!this.toc)` would skip the NCX fallback
        // and the reader ends up with an unusable empty TOC. Detect this
        // case by recursively checking whether any item has a real href.
        const hasNavigableHref = items => Array.isArray(items) && items.some(
            it => (it && (it.href || hasNavigableHref(it.subitems))))
        if (!hasNavigableHref(this.toc) && ncxPath) try {
            const resolve = url => resolveURL(url, ncxPath)
            const ncx = parseNCX(await this.#loadXML(ncxPath), resolve)
            this.toc = ncx.toc
            this.pageList = ncx.pageList
        } catch(e) {
            console.warn(e)
        }

        this.landmarks ??= this.resources.guide

        const { metadata, rendition, media } = getMetadata(opf)
        this.metadata = metadata
        this.rendition = rendition
        this.media = media
        this.dir = this.resources.pageProgressionDirection
        const displayOptions = getDisplayOptions(
            await this.#loadXML('META-INF/com.apple.ibooks.display-options.xml')
            ?? await this.#loadXML('META-INF/com.kobobooks.display-options.xml'))
        if (displayOptions) {
            if (displayOptions.fixedLayout === 'true')
                this.rendition.layout ??= 'pre-paginated'
            if (displayOptions.openToSpread === 'false') this.sections
                .find(section => section.linear !== 'no').pageSpread ??=
                    this.dir === 'rtl' ? 'left' : 'right'
        }
        return this
    }
    async loadDocument(item) {
        const str = await this.loadText(item.href)
        return parseContentDocument(this.parser, str, item.mediaType).doc
    }
    getMediaOverlay() {
        return new MediaOverlay(this, this.#loadXML.bind(this))
    }
    resolveCFI(cfi) {
        return this.resources.resolveCFI(cfi)
    }
    resolveHref(href) {
        const [path, hash] = href.split('#')
        const item = this.resources.getItemByHref(decodeURI(path))
        if (!item) return null
        const index = this.resources.spine.findIndex(({ idref }) => idref === item.id)
        const anchor = hash ? doc => getHTMLFragment(doc, hash) : () => 0
        return { index, anchor }
    }
    splitTOCHref(href) {
        return href?.split('#') ?? []
    }
    getTOCFragment(doc, id) {
        return doc.getElementById(id)
            ?? doc.querySelector(`[name="${CSS.escape(id)}"]`)
    }
    isExternal(uri) {
        return isExternal(uri)
    }
    async getCover() {
        const cover = this.resources?.cover
        if (cover?.href) return new Blob([await this.loadBlob(cover.href)],
            { type: cover.mediaType })
        // Fall back to a cover-named container entry. Some EPUBs ship the
        // cover image without ever declaring it (no `cover-image` property,
        // no `<meta name="cover">` target, no manifest item), which leaves
        // every manifest-driven lookup above empty even though the image is
        // sitting right there in the zip.
        const href = findUndeclaredCover(this.entries.keys())
        if (!href) return null
        const blob = await this.loadBlob(href)
        return blob ? new Blob([blob], { type: getImageMediaType(href) }) : null
    }
    async getCalibreBookmarks() {
        const txt = await this.loadText('META-INF/calibre_bookmarks.txt')
        const magic = 'encoding=json+base64:'
        if (txt?.startsWith(magic)) {
            const json = atob(txt.slice(magic.length))
            return JSON.parse(json)
        }
    }
    destroy() {
        this.#loader?.destroy()
    }
}

// Standalone OPF metadata extractor.
//
// Exposed so callers that already have the OPF bytes in hand (e.g. a
// platform-native pre-parser that read the zip on a faster runtime)
// can derive `Book.metadata` without driving the full `EPUB.init()` —
// which would force `@zip.js/zip.js` to scan the central directory
// and inflate nav.xhtml/ncx the importer never reads. The output
// shape is identical to what `EPUB.init()` would produce, so the
// import-path BookDoc and the reader-path BookDoc remain byte-stable
// across `Book.metadata.identifier`, title, contributors, refines
// chains, ONIX5, and `belongs-to-collection`.
//
// Two entry points to fit different callers:
//   - `getEpubMetadata(opfDoc)`        — already-parsed OPF Document
//   - `parseEpubMetadataFromXML(xml)`  — raw OPF XML string
export const getEpubMetadata = opf => getMetadata(opf)
export const parseEpubMetadataFromXML = xml => {
    const opf = new DOMParser().parseFromString(sanitizeXMLEntities(xml), 'application/xml')
    return getMetadata(opf)
}
