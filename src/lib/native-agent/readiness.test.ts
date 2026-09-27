import {afterEach, beforeEach, describe, expect, it, vi} from 'vitest';
vi.mock('../stores/download-listener',()=>({queueDataReady:()=>true}));
import {surfaceReadiness} from './readiness';
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
});
