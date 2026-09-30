import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
const queue=vi.hoisted(()=>({ready:true}));
vi.mock('../stores/download-listener',()=>({queueDataReady:()=>queue.ready}));
import {routeDataReady, surfaceReadiness} from './readiness';
const box=()=>({width:10,height:10,top:0,left:0,right:10,bottom:10});
let surfaces: {getBoundingClientRect:typeof box;getAttribute:()=>string}[];
let canvases: {getBoundingClientRect:typeof box;getAttribute:()=>string}[];
let doc: {images:unknown[];readyState:string;fonts:{status:string};querySelector:()=>unknown;querySelectorAll:(selector:string)=>unknown[]};
beforeEach(()=>{
 surfaces=[];canvases=[];
 doc={images:[],readyState:'complete',fonts:{status:'loaded'},querySelector:()=>null,querySelectorAll:s=>s==='canvas'?canvases:surfaces};
 vi.stubGlobal('document',doc);vi.stubGlobal('innerWidth',100);vi.stubGlobal('innerHeight',100);vi.stubGlobal('location',{pathname:'/downloads'});
});
afterEach(()=>vi.unstubAllGlobals());
describe('native surface readiness',()=>{
 it('blocks navigation, fonts, pending images and unregistered renderers',()=>{
  expect(surfaceReadiness(false,true,{}).readiness).toBe('loading');
  doc.fonts.status='loading';expect(surfaceReadiness(false,false,{}).readiness).toBe('loading');
  doc.fonts.status='loaded';doc.images=[{getBoundingClientRect:box,complete:false}];expect(surfaceReadiness(false,false,{}).readiness).toBe('loading');
  doc.images=[];canvases=[{getBoundingClientRect:box,getAttribute:()=> 'world'}];
  expect(surfaceReadiness(false,false,{}).readiness).toBe('loading');
  expect(surfaceReadiness(false,false,{other:{readiness:'ready'}}).readiness).toBe('loading');
  expect(surfaceReadiness(false,false,{world:{readiness:'ready'}}).readiness).toBe('ready');
 });
 it('exposes streaming and errors while preserving sensitive-surface blocking',()=>{
  surfaces=[{getBoundingClientRect:box,getAttribute:()=> 'streaming'}];
  expect(surfaceReadiness(false,false,{}).readiness).toBe('streaming');
  expect(surfaceReadiness(true,false,{}).readiness).toBe('blocked');
  expect(surfaceReadiness(false,false,{world:{readiness:'error'}}).readiness).toBe('error');
  surfaces=[];doc.images=[{getBoundingClientRect:box,complete:true,naturalWidth:0}];
  expect(surfaceReadiness(false,false,{}).readiness).toBe('error');
 });
 it('holds fetched routes until their surface settles: pending, error and empty ready',()=>{
  for (const path of ['/music','/about/changelog','/about/changelog/']) {
   vi.stubGlobal('location',{pathname:path});
   surfaces=[];
   let result=surfaceReadiness(false,false,{});
   expect(result.readiness).toBe('loading');
   expect(result.readinessDetails.dataReady).toBe(false);
   surfaces=[{getBoundingClientRect:box,getAttribute:()=> 'loading'}];
   result=surfaceReadiness(false,false,{});
   expect(result.readiness).toBe('loading');
   expect(result.readinessDetails.dataReady).toBe(false);
   surfaces=[{getBoundingClientRect:box,getAttribute:()=> 'error'}];
   result=surfaceReadiness(false,false,{});
   expect(result.readiness).toBe('error');
   expect(result.readinessDetails.dataReady).toBe(true);
   // An empty list that loaded is ready, not loading and not an error.
   surfaces=[{getBoundingClientRect:box,getAttribute:()=> 'ready'}];
   result=surfaceReadiness(false,false,{});
   expect(result.readiness).toBe('ready');
   expect(result.readinessDetails.dataReady).toBe(true);
   expect(surfaceReadiness(true,false,{}).readiness).toBe('blocked');
  }
 });
 it('keeps sensitive blocking while data is still pending',()=>{
  vi.stubGlobal('location',{pathname:'/music'});
  expect(surfaceReadiness(true,false,{}).readiness).toBe('blocked');
  vi.stubGlobal('location',{pathname:'/downloads'});queue.ready=false;
  expect(surfaceReadiness(true,false,{}).readiness).toBe('blocked');
  expect(surfaceReadiness(false,false,{}).readiness).toBe('loading');
  queue.ready=true;
 });
 it('leaves routes without a fetched-data contract to the other signals',()=>{
  expect(routeDataReady('/settings',[])).toBe(true);
  expect(routeDataReady('/music',[null])).toBe(false);
  expect(routeDataReady('/music',['loading','ready'])).toBe(true);
 });
});
