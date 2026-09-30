// The 3D city is a canvas, a GL context and Tauri commands: nothing to render
// on a server, and prerendering would only bake an empty <canvas>.
export const ssr = false;
export const prerender = false;
