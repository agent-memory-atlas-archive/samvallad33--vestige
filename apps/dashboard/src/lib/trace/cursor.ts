/**
 * The one dashboard cursor's "hot" state: true while a memory or organ drawn on
 * a WebGPU canvas is under the pointer. The DOM cannot know that (the canvas is
 * one element), so the page that picked it raises the flag, and the cursor
 * opens into its ring exactly as it does over a link or a button.
 */
import { writable } from 'svelte/store';

export const cursorHot = writable(false);
