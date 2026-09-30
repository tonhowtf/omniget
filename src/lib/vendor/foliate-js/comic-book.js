// @ts-nocheck
// Read series metadata from a ComicInfo.xml entry, if present.
// Spec: https://anansi-project.github.io/docs/comicinfo/intro
const readComicInfoXML = async ({ entries, loadBlob }) => {
    const entry = entries.find(e => e.filename.toLowerCase() === 'comicinfo.xml')
        ?? entries.find(e => e.filename.split('/').pop()?.toLowerCase() === 'comicinfo.xml')
    if (!entry) return null
    let text
    try {
        text = await (await loadBlob(entry.filename)).text()
    } catch {
        return null
    }
    let doc
    try {
        doc = new DOMParser().parseFromString(text, 'application/xml')
    } catch {
        return null
    }
    if (!doc || doc.getElementsByTagName('parsererror').length) return null
    const get = name => doc.getElementsByTagName(name).item(0)?.textContent?.trim() || undefined
    const getPositiveInteger = name => {
        const value = Number.parseInt(get(name), 10)
        return Number.isFinite(value) && value > 0 ? value : undefined
    }
    const getSubjects = () => [...new Set([get('Genre'), get('Tags')]
        .flatMap(value => value?.split(/[,;|]/).map(x => x.trim()).filter(Boolean) ?? []))]
    const getPublished = () => {
        const year = getPositiveInteger('Year')
        if (!year) return undefined
        const month = getPositiveInteger('Month')
        const day = getPositiveInteger('Day')
        const yyyy = String(year).padStart(4, '0')
        if (!month || month > 12) return yyyy
        const yyyyMm = `${yyyy}-${String(month).padStart(2, '0')}`
        if (!day || day > 31) return yyyyMm
        return `${yyyyMm}-${String(day).padStart(2, '0')}`
    }
    const subjects = getSubjects()
    return {
        title: get('Title'),
        publisher: get('Publisher'),
        language: get('LanguageISO'),
        author: get('Writer'),
        published: getPublished(),
        description: get('Summary'),
        subject: subjects.length ? subjects : undefined,
        identifier: get('Web'),
        series: get('Series'),
        seriesPosition: get('Number'),
        seriesTotal: get('Count'),
    }
}

const readComicBookInfo = async ({ getComment }) => {
    let info
    try {
        info = JSON.parse(await getComment() || '')['ComicBookInfo/1.0']
    } catch {
        return null
    }
    if (!info) return null
    const year = info.publicationYear
    const month = info.publicationMonth
    const mm = month && month >= 1 && month <= 12 ? String(month).padStart(2, '0') : null
    return {
        title: info.title,
        publisher: info.publisher,
        language: info.language || info.lang,
        author: info.credits ? info.credits.map(c => `${c.person} (${c.role})`).join(', ') : '',
        published: year && month ? `${year}-${mm}` : undefined,
        series: info.series,
        seriesPosition: info.issue == null ? undefined : String(info.issue),
    }
}

export const makeComicBook = async ({ entries, loadBlob, getSize, getComment }, file) => {
    const cache = new Map()
    const urls = new Map()
    const load = async name => {
        if (cache.has(name)) return cache.get(name)
        const src = URL.createObjectURL(await loadBlob(name))
        const page = URL.createObjectURL(
            new Blob([`<!DOCTYPE html><html><head><meta charset="utf-8"></head><body style="margin: 0"><img src="${src}"></body></html>`], { type: 'text/html' }))
        urls.set(name, [src, page])
        cache.set(name, page)
        return page
    }
    const unload = name => {
        urls.get(name)?.forEach?.(url => URL.revokeObjectURL(url))
        urls.delete(name)
        cache.delete(name)
    }

    const exts = ['.jpg', '.jpeg', '.png', '.gif', '.bmp', '.webp', '.svg', '.jxl', '.avif']
    // Compare paths segment by segment: sorting whole paths as strings puts
    // "Chapter 0060 (2)/001.jpg" before "Chapter 0060/001.jpg" (a space sorts
    // before a slash), showing part 2 of a split chapter first. Numeric
    // collation also keeps "2.jpg" ahead of "10.jpg".
    const collator = new Intl.Collator([], { numeric: true })
    const comparePaths = (a, b) => {
        const as = a.split('/'), bs = b.split('/')
        const len = Math.min(as.length, bs.length)
        for (let i = 0; i < len; i++) {
            const cmp = collator.compare(as[i], bs[i])
            if (cmp) return cmp
        }
        return as.length - bs.length
    }
    const files = entries
        .map(entry => entry.filename)
        .filter(name => exts.some(ext => name.endsWith(ext)))
        .sort(comparePaths)
    if (!files.length) throw new Error('No supported image files in archive')

    const book = {}
    // Prefer ComicInfo.xml (Anansi standard) over ComicBookInfo (JSON in zip comment).
    // Fields missing from the preferred source fall through to the secondary one.
    const xml = await readComicInfoXML({ entries, loadBlob })
    const cbi = await readComicBookInfo({ getComment })
    const merged = { ...(cbi || {}), ...(xml || {}) }
    book.metadata = {
        title: merged.title || file.name,
        publisher: merged.publisher,
        language: merged.language,
        author: merged.author,
        published: merged.published,
        description: merged.description,
        subject: merged.subject,
        identifier: merged.identifier,
    }
    if (merged.series) {
        const series = { name: merged.series }
        if (merged.seriesPosition) series.position = merged.seriesPosition
        if (merged.seriesTotal) series.total = merged.seriesTotal
        book.metadata.belongsTo = { series }
    }
    book.getCover = () => loadBlob(files[0])
    book.sections = files.map(name => ({
        id: name,
        load: () => load(name),
        unload: () => unload(name),
        size: getSize(name),
    }))
    book.toc = files.map(name => ({ label: name, href: name }))
    book.rendition = { layout: 'pre-paginated' }
    book.resolveHref = href => ({ index: book.sections.findIndex(s => s.id === href) })
    book.splitTOCHref = href => [href, null]
    book.getTOCFragment = doc => doc.documentElement
    book.destroy = () => {
        for (const arr of urls.values())
            for (const url of arr) URL.revokeObjectURL(url)
    }
    return book
}
