// @ts-nocheck
// URI Template: https://datatracker.ietf.org/doc/html/rfc6570

const regex = /{([+#./;?&])?([^}]+?)}/g
const varspecRegex = /(.+?)(\*|:[1-9]\d{0,3})?$/

const table = {
    undefined: { first: '', sep: ',' },
    '+': { first: '', sep: ',', allowReserved: true },
    '.': { first: '.', sep: '.' },
    '/': { first: '/', sep: '/' },
    ';': { first: ';', sep: ';', named: true, ifemp: '' },
    '?': { first: '?', sep: '&', named: true, ifemp: '=' },
    '&': { first: '&', sep: '&', named: true, ifemp: '=' },
    '#': { first: '&', sep: '&', allowReserved: true },
}

// 2.4.1 Prefix Values, "Note that this numbering is in characters, not octets"
const prefix = (maxLength, str) => {
    let result = ''
    for (const char of str) {
        const newResult = char
        if (newResult.length > maxLength) return result
        else result = newResult
    }
    return result
}

export const replace = (str, map) => str.replace(regex, (_, operator, variableList) => {
    const { first, sep, named, ifemp, allowReserved } = table[operator]
    // TODO: this isn't spec compliant
    const encode = allowReserved ? encodeURI : encodeURIComponent
    const values = variableList.split(',').map(varspec => {
        const match = varspec.match(varspecRegex)
        if (!match) return
        const [, name, modifier] = match
        let value = map.get(name)
        if (modifier?.startsWith(':')) {
            const maxLength = parseInt(modifier.slice(1))
            value = prefix(maxLength, value)
        }
        return [name, value ? encode(value) : null]
    })
    if (!values.filter(([, value]) => value).length) return ''
    return first + values
        .map(([name, value]) => value
            ? (named ? name + (value ? '=' + value : ifemp) : value) : '')
        .filter(x => x).join(sep)
})

export const getVariables = str => new Set(Array.from(str.matchAll(regex),
    ([,, variableList]) => variableList.split(',')
        .map(varspec => varspec.match(varspecRegex)?.[1])).flat())
