// Guard is a WebGPU client-only surface: it boots a GPU device on mount and
// reads the viewer's own gate receipts. SSR can never render it.
export const ssr = false;
export const prerender = false;
