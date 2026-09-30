// @ts-nocheck
const NS = {
    ATOM: 'http://www.w3.org/2005/Atom',
    OPDS: 'http://opds-spec.org/2010/catalog',
    THR: 'http://purl.org/syndication/thread/1.0',
    DC: 'http://purl.org/dc/elements/1.1/',
    DCTERMS: 'http://purl.org/dc/terms/',
    FH: 'http://purl.org/syndication/history/1.0',
    PSE: 'http://vaemendis.net/opds-pse/ns',
    OS: 'http://a9.com/-/spec/opensearch/1.1/',
}

const MIME = {
    ATOM: 'application/atom+xml',
    OPDS2: 'application/opds+json',
}

export const REL = {
    ACQ: 'http://opds-spec.org/acquisition',
    FACET: 'http://opds-spec.org/facet',
    GROUP: 'http://opds-spec.org/group',
    COVER: [
        'http://opds-spec.org/image',
        'http://opds-spec.org/cover', // ManyBooks legacy, not in spec
        'x-stanza-cover-image', // Lexcycle Stanza legacy
    ],
    THUMBNAIL: [
        'http://opds-spec.org/image/thumbnail',
        'http://opds-spec.org/thumbnail', // ManyBooks legacy, not in spec
        'x-stanza-cover-image-thumbnail', // Lexcycle Stanza legacy
    ],
    STREAM: 'http://vaemendis.net/opds-pse/stream',
}

export const SYMBOL = {
    SUMMARY: Symbol('summary'),
    CONTENT: Symbol('content'),
}

const FACET_GROUP = Symbol('facetGroup')

const groupByArray = (arr, f) => {
    const map = new Map()
    if (arr) {
        for (const el of arr) {
            const keys = f(el)
            const keyArray = keys == null ? [] : (Array.isArray(keys) ? keys : [keys])
            for (const key of keyArray) {
                const group = map.get(key)
                if (group) group.push(el)
                else map.set(key, [el])
            }
        }
    }
    return map
}

// https://www.rfc-editor.org/rfc/rfc7231#section-3.1.1
const parseMediaType = str => {
    if (!str) return
    const [mediaType, ...ps] = str.split(/ *; */)
    if (!mediaType) return
    return {
        mediaType: mediaType.toLowerCase(),
        parameters: ps.reduce((acc, p) => {
            const [name, val] = p.split('=')
            if (name) {
                acc[name.toLowerCase()] = val?.replace(/(^"|"$)/g, '')
            }
            return acc
        }, {}),
    }
}

export const isOPDSCatalog = str => {
    const parsed = parseMediaType(str)
    if (!parsed) return false
    const { mediaType, parameters } = parsed
    if (mediaType === MIME.OPDS2) return true
    return mediaType === MIME.ATOM && parameters.profile?.toLowerCase() === 'opds-catalog'
}

// ignore the namespace if it doesn't appear in document at all
const useNS = (doc, ns) =>
    doc.lookupNamespaceURI(null) === ns || doc.lookupPrefix(ns) ? ns : undefined

const filterNS = ns => ns
    ? name => el => el.namespaceURI === ns && el.localName === name
    : name => el => el.localName === name

const getContent = el => {
    if (!el) return
    const type = el.getAttribute('type') ?? 'text'
    const value = type === 'xhtml' ? el.innerHTML
        : type === 'html' ? el.textContent
            .replaceAll('&lt;', '<')
            .replaceAll('&gt;', '>')
            .replaceAll('&amp;', '&')
        : el.textContent
    return { value, type }
}

const getTextContent = el => {
    const content = getContent(el)
    if (content?.type === 'text') return content.value
}

const getSummary = (a, b) => getTextContent(a) ?? getTextContent(b)

// Fetch only direct children to avoid polluting with nested deep indirect acquisitions
const getDirectChildren = (el, ns, localName, tagName) => {
    return Array.from(el.childNodes).filter(node =>
        node.nodeType === 1 &&
        (
            (node.namespaceURI === ns && node.localName === localName) ||
            (node.tagName === tagName)
        )
    )
}

const getPrice = link => {
    const prices = getDirectChildren(link, NS.OPDS, 'price', 'opds:price')
    if (!prices.length) return
    const parsed = prices.reduce((acc, price) => {
        const value = parseFloat(price.textContent)
        if (!Number.isNaN(value)) {
            acc.push({
                currency: price.getAttribute('currencycode') ?? undefined,
                value,
            })
        }
        return acc
    }, [])

    if (!parsed.length) return
    // OPDS 1.x allows multiple prices, OPDS 2.0 schema defines price as a single object
    return parsed.length === 1 ? parsed[0] : parsed
}

const getIndirectAcquisition = el => {
    const ias = getDirectChildren(el, NS.OPDS, 'indirectAcquisition', 'opds:indirectAcquisition')
    if (!ias.length) return []
    return ias.reduce((acc, ia) => {
        const type = ia.getAttribute('type')
        if (type) {
            acc.push({
                type,
                child: getIndirectAcquisition(ia),
            })
        }
        return acc
    }, [])
}

const getLink = link => {
    const relAttr = link.getAttribute('rel')
    const rel = relAttr ? relAttr.split(/ +/) : undefined

    const isAcquisition = rel?.some(r => r.startsWith(REL.ACQ) || r === 'preview')
    const isStream = rel?.includes(REL.STREAM)

    // Map OPDS 1.x active facets to OPDS 2.0 "self" link
    const activeFacet = link.getAttributeNS(NS.OPDS, 'activeFacet') || link.getAttribute('opds:activeFacet')
    const mappedRel = activeFacet === 'true' ? [rel ?? []].flat().concat('self') : rel

    // Maps OPDS 1.x thr:count seamlessly to OPDS 2.0 properties.numberOfItems
    const thrCount = link.getAttributeNS(NS.THR, 'count') || link.getAttribute('thr:count')
    // Support for systems that incorrectly use standard `count` for facet hints
    const fallbackCount = link.getAttribute('count')

    // --- OPDS-PSE Extensions ---
    const pseCount = link.getAttributeNS(NS.PSE, 'count') || link.getAttribute('pse:count')
    const pseLastRead = link.getAttributeNS(NS.PSE, 'lastRead') || link.getAttribute('pse:lastRead')
    const pseLastReadDate = link.getAttributeNS(NS.PSE, 'lastReadDate') || link.getAttribute('pse:lastReadDate')

    return {
        rel: mappedRel,
        href: link.getAttribute('href') ?? undefined,
        type: link.getAttribute('type') ?? undefined,
        title: link.getAttribute('title') ?? undefined,
        // --- Facet Grouping ---
        [FACET_GROUP]: (link.getAttributeNS(NS.OPDS, 'facetGroup') || link.getAttribute('opds:facetGroup')) ?? undefined,
        properties: {
            price: (isAcquisition || isStream) ? getPrice(link) : undefined,
            indirectAcquisition: (isAcquisition || isStream) ? getIndirectAcquisition(link) : undefined,
            // --- Pagination / Facet Counters ---
            numberOfItems: thrCount != null ? Number(thrCount) : (!isStream && fallbackCount != null) ? Number(fallbackCount) : undefined,
            'pse:count': isStream && (pseCount ?? fallbackCount) != null ? Number(pseCount ?? fallbackCount) : undefined,
            'pse:lastRead': isStream && pseLastRead != null ? Number(pseLastRead) : undefined,
            'pse:lastReadDate': isStream ? pseLastReadDate ?? undefined : undefined,
        },
    }
}

const getPerson = person => {
    const NS = person.namespaceURI
    const uri = person.getElementsByTagNameNS(NS, 'uri')[0]?.textContent
    return {
        name: person.getElementsByTagNameNS(NS, 'name')[0]?.textContent ?? undefined,
        links: uri ? [{ href: uri }] : [],
    }
}

export const getPublication = entry => {
    const filter = filterNS(useNS(entry.ownerDocument, NS.ATOM))
    const children = Array.from(entry.children)
    const filterDCEL = filterNS(NS.DC)
    const filterDCTERMS = filterNS(NS.DCTERMS)
    const filterDC = x => y => filterDCEL(x)(y) || filterDCTERMS(x)(y)
    const links = children.filter(filter('link')).map(getLink)
    const linksByRel = groupByArray(links, link => link.rel)
    return {
        metadata: {
            id: children.find(filter('id'))?.textContent ?? undefined,
            updated: children.find(filter('updated'))?.textContent ?? undefined,
            title: children.find(filter('title'))?.textContent ?? undefined,
            author: children.filter(filter('author')).map(getPerson),
            contributor: children.filter(filter('contributor')).map(getPerson),
            publisher: children.find(filterDC('publisher'))?.textContent ?? undefined,
            published: (children.find(filterDCTERMS('issued'))
                ?? children.find(filterDC('date'))
                ?? children.find(filter('published')))?.textContent ?? undefined,
            language: children.find(filterDC('language'))?.textContent ?? undefined,
            identifier: children.find(filterDC('identifier'))?.textContent ?? undefined,
            subject: children.filter(filter('category')).map(category => ({
                name: category.getAttribute('label') ?? undefined,
                code: category.getAttribute('term') ?? undefined,
                scheme: category.getAttribute('scheme') ?? undefined,
            })),
            rights: children.find(filter('rights'))?.textContent ?? undefined,
            [SYMBOL.CONTENT]: getContent(children.find(filter('content')) ?? children.find(filter('summary'))) ?? undefined,
        },
        links,
        images: REL.COVER.concat(REL.THUMBNAIL)
            .map(R => linksByRel.get(R)?.[0]).filter(Boolean),
    }
}

export const getFeed = doc => {
    const ns = useNS(doc, NS.ATOM)
    const filter = filterNS(ns)
    const children = Array.from(doc.documentElement.children)
    const entries = children.filter(filter('entry'))
    const links = children.filter(filter('link')).map(getLink)
    const linksByRel = groupByArray(links, link => link.rel)

    const filterFH = filterNS(NS.FH)
    const filterOS = filterNS(NS.OS)

    const groupedItems = new Map([[undefined, []]])
    const groupLinkMap = new Map()
    for (const entry of entries) {
        const children = Array.from(entry.children)
        const links = children.filter(filter('link')).map(getLink)
        const linksByRel = groupByArray(links, link => link.rel)
        const hasAcquisition = Array.from(linksByRel.keys())
            .some(rel => rel?.startsWith(REL.ACQ) || rel === 'preview' || rel === REL.STREAM)
        // An entry with neither an acquisition link nor a link to a sub-catalog,
        // but which carries a cover/thumbnail image, is a publication that simply
        // has no downloadable format (e.g. a Calibre book whose file was removed
        // but whose metadata is kept). Treat it as a publication so its metadata
        // is shown; otherwise it falls through to the navigation branch with the
        // cover image as its href, and tapping it tries to parse the image as a
        // feed (readest issue #4599).
        const hasNavLink = links.some(link => isOPDSCatalog(link.type))
        const hasImage = REL.COVER.concat(REL.THUMBNAIL).some(rel => linksByRel.has(rel))
        const isPub = hasAcquisition || (!hasNavLink && hasImage)

        const groupLinks = linksByRel.get(REL.GROUP) ?? linksByRel.get('collection')
        const groupLink = groupLinks?.length
            ? groupLinks.find(link => groupedItems.has(link.href)) ?? groupLinks[0] : undefined
        if (groupLink?.href && !groupLinkMap.has(groupLink.href)) {
            groupLinkMap.set(groupLink.href, groupLink)
        }

        const item = isPub
            ? getPublication(entry)
            : Object.assign(links.find(link => isOPDSCatalog(link.type)) ?? links[0] ?? {}, {
                title: children.find(filter('title'))?.textContent ?? undefined,
                [SYMBOL.SUMMARY]: getSummary(children.find(filter('summary')),
                    children.find(filter('content'))) ?? undefined,
            })

        // Flag the item with its resolved OPDS 2.0 type
        item.__type = isPub ? 'publication' : 'navigation'

        const arr = groupedItems.get(groupLink?.href)
        if (arr) arr.push(item)
        else groupedItems.set(groupLink.href, [item])
    }

    const [items, ...groups] = Array.from(groupedItems, ([key, groupItems]) => {
        // Separate publications and navigation items within the group
        const publications = []
        const navigation = []
        for (const item of groupItems) {
            if (item.__type === 'publication') {
                delete item.__type
                publications.push(item)
            } else {
                delete item.__type
                navigation.push(item)
            }
        }

        const groupContent = {}
        if (publications.length) groupContent.publications = publications
        if (navigation.length) groupContent.navigation = navigation

        if (key === undefined) return groupContent
        const link = groupLinkMap.get(key)
        return {
            metadata: {
                title: link?.title,
                numberOfItems: link?.properties?.numberOfItems,
            },
            links: [{ rel: 'self', href: link?.href, type: link?.type }],
            ...groupContent,
        }
    })

    // --- OPDS 2.0 Pagination (derived from OpenSearch / RFC 5005) ---
    const totalResults = children.find(filterOS('totalResults'))?.textContent
    const itemsPerPage = children.find(filterOS('itemsPerPage'))?.textContent
    const startIndex = children.find(filterOS('startIndex'))?.textContent

    let currentPage
    if (startIndex != null && itemsPerPage != null) {
        const start = Number(startIndex)
        const items = Number(itemsPerPage)
        // Resolves typical 1-based offset to a page number
        currentPage = Math.floor((start > 0 ? start - 1 : 0) / items) + 1
    }

    return {
        metadata: {
            id: children.find(filter('id'))?.textContent ?? undefined,
            updated: children.find(filter('updated'))?.textContent ?? undefined,
            title: children.find(filter('title'))?.textContent ?? undefined,
            subtitle: children.find(filter('subtitle'))?.textContent ?? undefined,
            numberOfItems: totalResults != null ? Number(totalResults) : undefined,
            itemsPerPage: itemsPerPage != null ? Number(itemsPerPage) : undefined,
            currentPage,
        },
        links,
        isComplete: !!children.find(filterFH('complete')) || undefined,
        isArchive: !!children.find(filterFH('archive')) || undefined,
        ...items, // contains top-level 'publications' and/or 'navigation' arrays
        groups: groups.length ? groups : undefined,
        facets: Array.from(
            groupByArray(linksByRel.get(REL.FACET) ?? [], link => link[FACET_GROUP]),
            ([facet, links]) => ({ metadata: { title: facet ?? undefined }, links })
        ),
    }
}

export const getSearch = async link => {
    const { replace, getVariables } = await import('./uri-template.js')
    const href = link.href || ''
    return {
        metadata: {
            title: link.title ?? undefined,
        },
        search: map => replace(href, map.get(undefined)),
        params: Array.from(getVariables(href), name => ({ name })),
    }
}

export const getOpenSearch = doc => {
    const defaultNS = doc.documentElement.namespaceURI
    const filter = filterNS(defaultNS)
    const children = Array.from(doc.documentElement.children)

    const $$urls = children.filter(filter('Url'))
    const $url = $$urls.find(url => isOPDSCatalog(url.getAttribute('type'))) ?? $$urls[0]
    if (!$url) throw new Error('document must contain at least one Url element')

    const regex = /{(?:([^}]+?):)?(.+?)(\?)?}/g
    const defaultMap = new Map([
        ['count', '100'],
        ['startIndex', $url.getAttribute('indexOffset') ?? '0'],
        ['startPage', $url.getAttribute('pageOffset') ?? '0'],
        ['language', '*'],
        ['inputEncoding', 'UTF-8'],
        ['outputEncoding', 'UTF-8'],
    ])

    const template = $url.getAttribute('template') || ''
    return {
        metadata: {
            title: (children.find(filter('LongName')) ?? children.find(filter('ShortName')))?.textContent ?? undefined,
            description: children.find(filter('Description'))?.textContent ?? undefined,
        },
        search: map => template.replace(regex, (_, prefix, param) => {
            const namespace = prefix ? $url.lookupNamespaceURI(prefix) : undefined
            const ns = namespace === defaultNS ? undefined : namespace
            const val = map.get(ns)?.get(param)
            return encodeURIComponent(val ?? (!ns ? defaultMap.get(param) ?? '' : ''))
        }),
        params: Array.from(template.matchAll(regex), ([, prefix, param, optional]) => {
            const namespace = prefix ? $url.lookupNamespaceURI(prefix) : undefined
            const ns = namespace === defaultNS ? undefined : namespace
            return {
                ns,
                name: param, // (.+?) ensures `param` is non-empty
                required: !optional,
                value: ns && ns !== defaultNS ? undefined : defaultMap.get(param) ?? undefined,
            }
        }),
    }
}
