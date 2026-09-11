// A DOM for the glyph tests, the way @qntx/glyphs sets up its own: happy-dom's
// window and document as globals.
const { Window } = await import('happy-dom')
const window = new Window()

// @ts-ignore
globalThis.window = window
// @ts-ignore
globalThis.document = window.document
// @ts-ignore
globalThis.HTMLElement = window.HTMLElement
// @ts-ignore
globalThis.Event = window.Event
// @ts-ignore
globalThis.localStorage = window.localStorage
// @ts-ignore
globalThis.MutationObserver = window.MutationObserver
