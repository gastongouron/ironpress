/* tslint:disable */
/* eslint-disable */

/**
 * Convert HTML to PDF bytes.
 *
 * Returns a `Uint8Array` containing the PDF document.
 */
export function htmlToPdf(html: string): Uint8Array;

/**
 * Convert HTML to PDF with custom page size and margins.
 *
 * `page_width` and `page_height` are in points (1 inch = 72 points).
 * `margin_top`, `margin_right`, `margin_bottom`, `margin_left` are in points.
 */
export function htmlToPdfCustom(html: string, page_width: number, page_height: number, margin_top: number, margin_right: number, margin_bottom: number, margin_left: number): Uint8Array;

/**
 * Convert Markdown to PDF bytes.
 *
 * Returns a `Uint8Array` containing the PDF document.
 */
export function markdownToPdf(md: string): Uint8Array;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly htmlToPdf: (a: number, b: number) => [number, number, number];
    readonly htmlToPdfCustom: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number, number];
    readonly markdownToPdf: (a: number, b: number) => [number, number, number];
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
