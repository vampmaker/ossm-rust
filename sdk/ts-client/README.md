# @ossm/client

Universal TypeScript / JavaScript client SDK and single-owner WebSocket manager for interacting with **OSSM (Open Source Sex Machine)** hardware devices over HTTP REST and WebSocket JSON-RPC 2.0.

## Features

- **Single-Owner WebSocket Manager (`WsDataManager`)**:
  - **Deferred Pattern**: Correlates JSON-RPC responses via IDs and manages suspended coroutines awaiting shared state updates.
  - **In-Flight Promise Mutex**: Prevents duplicate WebSocket connection handshakes under high concurrency.
  - **Debouncing Idle Timer**: Automatically closes idle connections after a configurable timeout when no active subscribers or pending requests remain.
  - **Shared Data Caching**: Serves cached device state instantly without network latency when data is fresh.
- **Universal SDK (`OssmClient`)**:
  - Fully typed REST and WebSocket interface for Node.js, desktop apps (Electron/Tauri), browser applications, or automation scripts.

## Usage Example

```typescript
import { OssmClient } from '@ossm/client'

const client = new OssmClient('http://192.168.24.63')

// Fetch and update motor controller configuration
const config = await client.getConfig()
console.log('Current BPM:', config.bpm)

// Retrieve live shared motor state
const state = await client.getSharedState()
console.log('Current Position:', state.position)

// Subscribe to real-time state telemetry pushed at 30 FPS
await client.subscribeState((st) => {
  console.log('Live UPS:', st.ups, 'dt avg ms:', st.dt_avg_ms)
}, 33)
```
