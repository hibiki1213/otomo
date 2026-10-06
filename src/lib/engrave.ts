/**
 * Engraving MusicXML into printable pages, for the PDF export.
 *
 * Verovio (LGPL-3.0-or-later, https://www.verovio.org) does the engraving,
 * in a worker (`engraveWorker.ts`): a large score takes seconds. Its wasm is
 * about 7 MB, so the worker starts the first time a score is engraved rather
 * than with the app. The pages come out as SVG; `pdf_write` in
 * `src-tauri/src/pdf.rs` turns them into a vector PDF.
 */

export type Paper = "a4" | "letter";

export interface EngraveOptions {
  paper: Paper;
  /** Full score with many staves wants a smaller staff size than a single part. */
  kind: "score" | "part";
  /** Write transposing instruments at sounding pitch (a concert-pitch score). */
  concertPitch?: boolean;
}

export interface Engraved {
  /** One SVG document per page, drawn edge to edge on the paper. */
  pages: string[];
  widthMm: number;
  heightMm: number;
}

/** What goes to the worker and back. */
export interface EngraveRequest {
  id: number;
  musicxml: string;
  opts: EngraveOptions;
}
export type EngraveReply = { id: number; engraved: Engraved } | { id: number; error: string };

/**
 * Whether this web view can run Verovio. Its wasm uses exception handling,
 * bulk memory and BigInt64Array: Safari 15.2 or later, which macOS 11 only
 * has once Safari has been updated.
 */
export function engraveSupported(): boolean {
  if (typeof WebAssembly !== "object" || typeof BigInt64Array === "undefined") return false;
  // The smallest module with a tag section validates only where wasm
  // exceptions do.
  const withTag = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 13, 3, 1, 0, 0]);
  try {
    return WebAssembly.validate(withTag);
  } catch {
    return false;
  }
}

let worker: Worker | undefined;
let lastId = 0;
const pending = new Map<number, { resolve: (e: Engraved) => void; reject: (e: Error) => void }>();

/** One worker for the app's lifetime, started on first use. */
function engraver(): Worker {
  if (worker) return worker;
  const w = new Worker(new URL("./engraveWorker.ts", import.meta.url), { type: "module" });
  w.onmessage = (e: MessageEvent<EngraveReply>) => {
    const reply = e.data;
    const call = pending.get(reply.id);
    pending.delete(reply.id);
    if ("error" in reply) call?.reject(new Error(reply.error));
    else call?.resolve(reply.engraved);
  };
  // The worker could not start or died: fail what is waiting, and start a
  // fresh one next time.
  w.onerror = (e) => {
    e.preventDefault();
    const error = new Error(e.message || "The engraving worker stopped");
    for (const call of pending.values()) call.reject(error);
    pending.clear();
    w.terminate();
    if (worker === w) worker = undefined;
  };
  return (worker = w);
}

/** Lays out `musicxml` on pages. Calls are engraved one at a time, in order. */
export function engrave(musicxml: string, opts: EngraveOptions): Promise<Engraved> {
  if (!engraveSupported()) return Promise.reject(new Error("Engraving needs Safari 15.2 or later"));
  return new Promise((resolve, reject) => {
    const id = ++lastId;
    pending.set(id, { resolve, reject });
    const request: EngraveRequest = { id, musicxml, opts };
    engraver().postMessage(request);
  });
}
