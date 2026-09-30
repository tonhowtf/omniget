// @ts-nocheck
const normalizeWhitespace = str => str ? str
    .replace(/[\t\n\f\r ]+/g, ' ')
    .replace(/^[\t\n\f\r ]+/, '')
    .replace(/[\t\n\f\r ]+$/, '') : ''
const getElementText = el => normalizeWhitespace(el?.textContent)

const NS = {
    XLINK: 'http://www.w3.org/1999/xlink',
    EPUB: 'http://www.idpf.org/2007/ops',
}

const MIME = {
    XML: 'application/xml',
    XHTML: 'application/xhtml+xml',
}

const STYLE = {
    'strong': ['strong', 'self'],
    'emphasis': ['em', 'self'],
    'style': ['span', 'self'],
    'a': 'anchor',
    'strikethrough': ['s', 'self'],
    'sub': ['sub', 'self'],
    'sup': ['sup', 'self'],
    'code': ['code', 'self'],
    'image': 'image',
}

const TABLE = {
    'tr': ['tr', {
        'th': ['th', STYLE, ['colspan', 'rowspan', 'align', 'valign']],
        'td': ['td', STYLE, ['colspan', 'rowspan', 'align', 'valign']],
    }, ['align']],
}

const POEM = {
    'epigraph': ['blockquote'],
    'subtitle': ['h2', STYLE],
    'text-author': ['p', STYLE],
    'date': ['p', STYLE],
    'stanza': ['div', 'self'],
    'v': ['div', STYLE],
}

const SECTION = {
    'title': ['header', {
        'p': ['h1', STYLE],
        'empty-line': ['br'],
    }],
    'epigraph': ['blockquote', 'self'],
    'image': 'image',
    'annotation': ['aside'],
    'section': ['section', 'self'],
    'p': ['p', STYLE],
    'poem': ['blockquote', POEM],
    'subtitle': ['h2', STYLE],
    'cite': ['blockquote', 'self'],
    'empty-line': ['br'],
    'table': ['table', TABLE],
    'text-author': ['p', STYLE],
}
POEM['epigraph'].push(SECTION)

const BODY = {
    'image': 'image',
    'title': ['section', {
        'p': ['h1', STYLE],
        'empty-line': ['br'],
    }],
    'epigraph': ['section', SECTION],
    'section': ['section', SECTION],
}

class FB2Converter {
    constructor(fb2) {
        this.fb2 = fb2
        this.doc = document.implementation.createDocument(NS.XHTML, 'html')
        // use this instead of `getElementById` to allow images like
        // `<image l:href="#img1.jpg" id="img1.jpg" />`
        this.bins = new Map(Array.from(this.fb2.getElementsByTagName('binary'),
            el => [el.id, el]))
    }
    getImageSrc(el) {
        const href = el.getAttributeNS(NS.XLINK, 'href')
        if (!href) return 'data:,'
        const [, id] = href.split('#')
        if (!id) return href
        const bin = this.bins.get(id)
        return bin
            ? `data:${bin.getAttribute('content-type')};base64,${bin.textContent}`
            : href
    }
    image(node) {
        const el = this.doc.createElement('img')
        el.alt = node.getAttribute('alt')
        el.title = node.getAttribute('title')
        el.setAttribute('src', this.getImageSrc(node))
        return el
    }
    anchor(node) {
        const el = this.convert(node, { 'a': ['a', STYLE] })
        el.setAttribute('href', node.getAttributeNS(NS.XLINK, 'href'))
        if (node.getAttribute('type') === 'note')
            el.setAttributeNS(NS.EPUB, 'epub:type', 'noteref')
        return el
    }
    convert(node, def) {
        // not an element; return text content
        if (node.nodeType === 3) return this.doc.createTextNode(node.textContent)
        if (node.nodeType === 4) return this.doc.createCDATASection(node.textContent)
        if (node.nodeType === 8) return this.doc.createComment(node.textContent)

        const d = def?.[node.nodeName]
        if (!d) return null
        if (typeof d === 'string') return this[d](node)

        const [name, opts, attrs] = d
        const el = this.doc.createElement(name)

        // copy the ID, and set class name from original element name
        if (node.id) el.id = node.id
        el.classList.add(node.nodeName)

        // copy attributes
        if (Array.isArray(attrs)) for (const attr of attrs) {
            const value = node.getAttribute(attr)
            if (value) el.setAttribute(attr, value)
        }

        // process child elements recursively
        const childDef = opts === 'self' ? def : opts
        let child = node.firstChild
        while (child) {
            const childEl = this.convert(child, childDef)
            if (childEl) el.append(childEl)
            child = child.nextSibling
        }
        return el
    }
}

const parseXML = async blob => {
    const buffer = await blob.arrayBuffer()
    const str = new TextDecoder('utf-8').decode(buffer)
    const parser = new DOMParser()
    const doc = parser.parseFromString(str, MIME.XML)
    const encoding = doc.xmlEncoding
        // `Document.xmlEncoding` is deprecated, and already removed in Firefox
        // so parse the XML declaration manually
        || str.match(/^<\?xml\s+version\s*=\s*["']1.\d+"\s+encoding\s*=\s*["']([A-Za-z0-9._-]*)["']/)?.[1]
    if (encoding && encoding.toLowerCase() !== 'utf-8') {
        const str = new TextDecoder(encoding).decode(buffer)
        return parser.parseFromString(str, MIME.XML)
    }
    return doc
}

const style = URL.createObjectURL(new Blob([`
@namespace epub "http://www.idpf.org/2007/ops";
body > img, section > img {
    display: block;
    margin: auto;
}
.title h1 {
    text-align: center;
}
body > section > .title, body.notesBodyType > .title {
    margin: 3em 0;
}
body.notesBodyType > section .title h1 {
    text-align: start;
}
body.notesBodyType > section .title {
    margin: 1em 0;
}
p {
    text-indent: 1em;
    margin: 0;
}
:not(p) + p, p:first-child {
    text-indent: 0;
}
.stanza {
    text-indent: 0;
    margin: 1em 0;
}
.text-author, .date {
    text-align: end;
}
.text-author:before {
    content: "—";
}
table {
    border-collapse: collapse;
}
td, th {
    padding: .25em;
}
a[epub|type~="noteref"] {
    font-size: .75em;
    vertical-align: super;
}
body:not(.notesBodyType) > .title, body:not(.notesBodyType) > .epigraph {
    margin: 3em 0;
}
`], { type: 'text/css' }))

const template = html => `<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
    <head><link href="${style}" rel="stylesheet" type="text/css"/></head>
    <body>${html}</body>
</html>`

// name of custom ID attribute for TOC items
const dataID = 'data-foliate-id'

export const makeFB2 = async blob => {
    const book = {}
    const doc = await parseXML(blob)
    const converter = new FB2Converter(doc)

    const $ = x => doc.querySelector(x)
    const $$ = x => [...doc.querySelectorAll(x)]
    const getPerson = el => {
        const nick = getElementText(el.querySelector('nickname'))
        if (nick) return nick
        const first = getElementText(el.querySelector('first-name'))
        const middle = getElementText(el.querySelector('middle-name'))
        const last = getElementText(el.querySelector('last-name'))
        const name = [first, middle, last].filter(x => x).join(' ')
        const sortAs = last
            ? [last, [first, middle].filter(x => x).join(' ')].join(', ')
            : null
        return { name, sortAs }
    }
    const getDate = el => el?.getAttribute('value') ?? getElementText(el)
    const annotation = $('title-info annotation')
    // FB2 stores series info as `<sequence name="…" number="…"/>` in title-info
    const series = $$('title-info > sequence')
        .map(el => ({
            name: normalizeWhitespace(el.getAttribute('name')),
            position: el.getAttribute('number') || undefined,
        }))
        .filter(x => x.name)
    book.metadata = {
        title: getElementText($('title-info book-title')),
        identifier: getElementText($('document-info id')),
        language: getElementText($('title-info lang')),
        author: $$('title-info author').map(getPerson),
        translator: $$('title-info translator').map(getPerson),
        contributor: $$('document-info author').map(getPerson)
            // techincially the program probably shouldn't get the `bkp` role
            // but it has been so used by calibre, so ¯\_(ツ)_/¯
            .concat($$('document-info program-used').map(getElementText))
            .map(x => Object.assign(typeof x === 'string' ? { name: x } : x,
                { role: 'bkp' })),
        belongsTo: series.length ? { series } : undefined,
        publisher: getElementText($('publish-info publisher')),
        published: getDate($('title-info date')),
        modified: getDate($('document-info date')),
        description: annotation ? converter.convert(annotation,
            { annotation: ['div', SECTION] }).innerHTML : null,
        subject: $$('title-info genre').map(getElementText),
    }
    if ($('coverpage image')) {
        const src = converter.getImageSrc($('coverpage image'))
        book.getCover = () => fetch(src).then(res => res.blob())
    } else book.getCover = () => null

    // get convert each body
    const bodyData = Array.from(doc.querySelectorAll('body'), body => {
        const converted = converter.convert(body, { body: ['body', BODY] })
        return [Array.from(converted.children, el => {
            // get list of IDs in the section
            const ids = [el, ...el.querySelectorAll('[id]')].map(el => el.id)
            return { el, ids }
        }), converted]
    })

    const urls = []
    const sectionData = bodyData[0][0]
        // make a separate section for each section in the first body
        .map(({ el, ids }, id) => {
            // set up titles for TOC
            const titles = Array.from(
                el.querySelectorAll(':scope > section > .title'),
                (el, index) => {
                    el.setAttribute(dataID, index)
                    const section = el.closest('section')
                    const size = new TextEncoder().encode(section.innerHTML).length
                        - Array.from(section.querySelectorAll('[src]'))
                            .reduce((sum, el) => sum + (el.getAttribute('src')?.length ?? 0), 0)
                    return { title: getElementText(el), index, size, href: `${id}#${index}` }
                })
            return { ids, titles, el }
        })
        // for additional bodies, only make one section for each body
        .concat(bodyData.slice(1).map(([sections, body]) => {
            const ids = sections.map(s => s.ids).flat()
            body.classList.add('notesBodyType')
            return { ids, el: body, linear: 'no' }
        }))
        .map(({ ids, titles, el, linear }) => {
            const str = template(el.outerHTML)
            const blob = new Blob([str], { type: MIME.XHTML })
            const url = URL.createObjectURL(blob)
            urls.push(url)
            const title = normalizeWhitespace(
                el.querySelector('.title, .subtitle, p')?.textContent
                ?? (el.classList.contains('title') ? el.textContent : ''))
            return {
                ids, title, titles, load: () => url,
                createDocument: () => new DOMParser().parseFromString(str, MIME.XHTML),
                // doo't count image data as it'd skew the size too much
                size: blob.size - Array.from(el.querySelectorAll('[src]'),
                    el => el.getAttribute('src')?.length ?? 0)
                    .reduce((a, b) => a + b, 0),
                linear,
            }
        })

    const idMap = new Map()
    book.sections = sectionData.map((section, index) => {
        const { ids, load, createDocument, size, linear, titles } = section
        for (const id of ids) if (id) idMap.set(id, index)
        return { id: index, load, createDocument, size, linear, subitems: titles }
    })

    book.toc = sectionData.map(({ title, titles }, index) => {
        const id = index.toString()
        return {
            label: title,
            href: id,
            subitems: titles?.length ? titles.map(({ title, index }) => ({
                label: title,
                href: `${id}#${index}`,
            })) : null,
        }
    }).filter(item => item)

    book.resolveHref = href => {
        const [a, b] = href.split('#')
        return a
            // the link is from the TOC
            ? { index: Number(a), anchor: doc => doc.querySelector(`[${dataID}="${b}"]`) }
            // link from within the page
            : { index: idMap.get(b), anchor: doc => doc.getElementById(b) }
    }
    book.splitTOCHref = href => href?.split('#')?.map(x => Number(x)) ?? []
    book.getTOCFragment = (doc, id) => doc.querySelector(`[${dataID}="${id}"]`)

    book.destroy = () => {
        for (const url of urls) URL.revokeObjectURL(url)
    }
    return book
}
