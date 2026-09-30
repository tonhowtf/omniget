// @ts-nocheck
const SVG_NS = 'http://www.w3.org/2000/svg'

// bisect
const fit = (el, a = 1, b = 50) => {
    const c = Math.floor(a + (b - a) / 2)
    el.style.fontSize = `${c}px`
    if (b - a === 1) return
    if (el.scrollHeight > el.clientHeight
    || el.scrollWidth > el.clientWidth) fit(el, a, c)
    else fit(el, c, b)
}

const width = 540
const height = 540
const pixelRatio = 2

const html = `<style>
:host {
    position: absolute;
    width: 0;
    height: 0;
    overflow: hidden;
    visibility: hidden;
}
</style>
<main style="width: ${width}px; height: ${height}px; overflow: hidden; display: flex; flex-direction: column; justify-content: center; text-align: center; background: #fff; color: #000; font: 16pt serif">
    <style>
    .ellipsis {
        overflow: hidden;
        display: -webkit-box;
        -webkit-box-orient: vertical;
        -webkit-line-clamp: 2;
        text-overflow: ellipsis;
    }
    </style>
    <div style="font-size: min(4em, 8rem); line-height: 1; margin-bottom: -.5em">“</div>
    <div id="text" style="margin: 1em; text-wrap: balance"></div>
    <div style="margin: 0 1em">
        <div style="font-size: min(.8em, 1.25rem); font-family: sans-serif">
            <div style="display: block; font-weight: bold; margin-bottom: .25em">
                <span id="author" class="ellipsis"></span>
            </div>
            <div style="display: block; text-wrap: balance">
                <cite id="title" class="ellipsis"></cite>
            </div>
        </div>
    </div>
    <div style="height: 1em">&nbsp;</div>
</main>`

// TODO: lang, vertical writing
customElements.define('foliate-quoteimage', class extends HTMLElement {
    #root = this.attachShadow({ mode: 'closed' })
    constructor() {
        super()
        this.#root.innerHTML = html
    }
    async getBlob({ title, author, text }) {
        this.#root.querySelector('#title').textContent = title
        this.#root.querySelector('#author').textContent = author
        this.#root.querySelector('#text').innerText = text

        fit(this.#root.querySelector('main'))

        const img = document.createElement('img')
        return new Promise(resolve => {
            img.onload = () => {
                const canvas = document.createElement('canvas')
                canvas.width = pixelRatio * width
                canvas.height = pixelRatio * height
                const ctx = canvas.getContext('2d')
                ctx.drawImage(img, 0, 0, canvas.width, canvas.height)
                canvas.toBlob(resolve)
            }
            const doc = document.implementation.createDocument(SVG_NS, 'svg')
            doc.documentElement.setAttribute('viewBox', `0 0 ${width} ${height}`)
            const obj = doc.createElementNS(SVG_NS, 'foreignObject')
            obj.setAttribute('width', width)
            obj.setAttribute('height', height)
            obj.append(doc.importNode(this.#root.querySelector('main'), true))
            doc.documentElement.append(obj)
            img.src = 'data:image/svg+xml;charset=utf-8,'
                + new XMLSerializer().serializeToString(doc)
        })
    }
})
