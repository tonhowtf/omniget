import { queueDataReady } from '../stores/download-listener';
const FETCHED_DATA_ROUTES: readonly string[] = ['/music', '/about/changelog'];
function routeDataReady(pathname: string, surfaceStates: readonly (string | null)[]): boolean {
 const path = pathname.replace(/\/+$/, '') || '/';
 if (path === '/downloads') return queueDataReady();
 if (FETCHED_DATA_ROUTES.includes(path)) return surfaceStates.some(s => s === 'ready' || s === 'error');
 return true;
}
export function surfaceReadiness(sensitive: boolean, navigating: boolean, semantics: Record<string, unknown>) {
 const visible=(el: Element)=>{const r=el.getBoundingClientRect();return r.width>0&&r.height>0&&r.bottom>0&&r.right>0&&r.top<innerHeight&&r.left<innerWidth;};
 const images=[...document.images].filter(visible);
 const pendingImages=images.filter(i=>!i.complete).length;
 const failedImages=images.filter(i=>i.complete&&i.naturalWidth===0).length;
 const surfaces=[...document.querySelectorAll('[data-agent-readiness]')].filter(visible);
 const surfaceStates=surfaces.map(e=>e.getAttribute('data-agent-readiness'));
 const states=[...surfaceStates, ...Object.values(semantics).flatMap(s=>s&&typeof s==='object'&&'readiness' in s?[String(s.readiness)]:[])];
 const canvas=[...document.querySelectorAll('canvas')].filter(visible);
 const rendererReady=canvas.every(el=>{const id=el.getAttribute('data-agent-renderer');const s=id?semantics[id]:null;return !!s&&typeof s==='object'&&(('ready' in s&&s.ready===true)||('readiness' in s&&s.readiness==='ready'));});
 const streaming=states.includes('streaming');
 const error=states.includes('error')||failedImages>0;
 const dataReady=routeDataReady(location.pathname, surfaceStates);
 const loading=!dataReady||navigating||document.readyState!=='complete'||document.fonts.status!=='loaded'||pendingImages>0||states.includes('loading')||!!document.querySelector('[aria-busy="true"]')||!rendererReady;
 return {readiness:sensitive?'blocked':error?'error':streaming?'streaming':loading?'loading':'ready',readinessDetails:{dataReady,navigation:!navigating,document:document.readyState,fonts:document.fonts.status,pendingImages,failedImages,rendererReady,streaming,error,surfaces:states}};
}
