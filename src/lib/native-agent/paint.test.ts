import { afterEach, expect, it, vi } from 'vitest';
import { waitForPaint } from './paint';
afterEach(()=>{vi.useRealTimers();vi.unstubAllGlobals();});
it('settles an occluded WebView even when no animation frame runs', async()=>{
  vi.useFakeTimers();const cancel=vi.fn();vi.stubGlobal('requestAnimationFrame',vi.fn(()=>42));vi.stubGlobal('cancelAnimationFrame',cancel);
  const settled=vi.fn();const pending=waitForPaint().then(settled);
  await vi.advanceTimersByTimeAsync(99);expect(settled).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1);await pending;expect(settled).toHaveBeenCalledOnce();expect(cancel).toHaveBeenCalledWith(42);
});
it('settles after two frames without retaining a timer', async()=>{
  vi.useFakeTimers();const callbacks:FrameRequestCallback[]=[];vi.stubGlobal('requestAnimationFrame',(cb:FrameRequestCallback)=>callbacks.push(cb));vi.stubGlobal('cancelAnimationFrame',vi.fn());
  const pending=waitForPaint();callbacks[0](0);callbacks[1](16);await pending;expect(vi.getTimerCount()).toBe(0);
});
