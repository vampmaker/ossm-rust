/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_OSSM_API?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
