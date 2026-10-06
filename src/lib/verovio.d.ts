// The verovio package ships no types; this covers what engrave.ts uses.

declare module "verovio/wasm" {
  /** Instantiates Verovio's Emscripten module (the wasm is inlined in it). */
  export default function createVerovioModule(moduleArg?: object): Promise<unknown>;
}

declare module "verovio/esm" {
  export class VerovioToolkit {
    constructor(module: unknown);
    setOptions(options: Record<string, unknown>): void;
    resetOptions(): void;
    loadData(data: string): boolean;
    getMEI(options?: Record<string, unknown>): string;
    getPageCount(): number;
    renderToSVG(pageNo?: number, xmlDeclaration?: boolean): string;
    getLog(): string;
    getVersion(): string;
  }
  export function enableLogToBuffer(value: boolean, module: unknown): void;
}
