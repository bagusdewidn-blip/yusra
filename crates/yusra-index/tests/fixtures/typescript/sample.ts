/** Modul contoh untuk uji ekstraksi simbol. Tidak dikompilasi. */

import { readFile } from "node:fs/promises";
import type { Config } from './config';
import * as path from "path";
export { helper } from "./helpers";
export * from "./types";

const legacy = require("./legacy");

/** Batas percobaan. */
export const MAX_RETRY = 3;

let counter = 0;

export const first = 1, second = 2;

/**
 * Session login.
 */
export interface Session {
  id: number;
  refresh(): boolean;
}

export type Id = string | number;

export enum State {
  Active,
  Closed,
}

// komentar biasa, bukan doc
export function makeToken(id: number): string {
  return `t${id}`;
}

export function parse(input: string): number;
export function parse(input: number): number;
export function parse(input: string | number): number {
  return Number(input);
}

/** Klien HTTP. */
@Injectable({ providedIn: "root" })
export class Client<T> extends Base implements Closeable {
  private retries = 0;

  constructor(private readonly url: string) {
    super();
  }

  /** Kirim permintaan. */
  async send(body: T): Promise<Response> {
    const res = await fetch(this.url, { body: JSON.stringify(body) });
    return res;
  }

  @Log()
  close(): void {
    this.retries = 0;
  }

  handle = (event: Event): void => {
    console.log(event);
  };
}

export abstract class Shape {
  abstract area(): number;
}

export const double = (x: number): number => x * 2;

export default function (): void {
  new Client<string>("https://example.com").close();
}

export namespace auth {
  export function login(): boolean {
    return makeToken(1).length > 0;
  }
}

declare module "virtual:config" {
  export const value: string;
}

async function lazy() {
  const mod = await import("./lazy");
  return mod;
}
