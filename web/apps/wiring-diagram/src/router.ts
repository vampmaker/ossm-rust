import { ref } from 'vue'
import type { DiagramRoute } from './types'

function getRouteFromHash(): DiagramRoute {
  const hash = window.location.hash.replace(/^#\/?/, '').trim()
  if (hash === 'std-zh' || hash === 'esp' || hash === 'esp-zh') {
    return hash
  }
  return 'std'
}

export const currentRoute = ref<DiagramRoute>(getRouteFromHash())

window.addEventListener('hashchange', () => {
  currentRoute.value = getRouteFromHash()
})

export function setRoute(route: DiagramRoute) {
  window.location.hash = `#/${route}`
}
