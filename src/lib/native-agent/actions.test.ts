import { expect, it } from 'vitest';
import { agentActions, allowedActions } from './actions';
it('rendered attributes never grant actions; component teardown revokes them', () => {
  const fake={dataset:{agentActions:'click'}} as unknown as HTMLElement;
  expect(allowedActions(fake)).toEqual([]);
  const cleanup=agentActions(fake,['scroll']);expect(allowedActions(fake)).toEqual(['scroll']);
  cleanup.destroy();expect(allowedActions(fake)).toEqual([]);
});
