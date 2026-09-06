/// <reference types="vite/client" />
/// <reference types="@types/w3c-web-serial" />

declare module '*.wasm?arraybuffer' {
  const bytes: ArrayBuffer
  export default bytes
}
