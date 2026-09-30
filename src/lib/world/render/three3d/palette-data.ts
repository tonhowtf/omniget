// The built-in palette, without three.js (the lite city reads it too).

import type { Palette } from '$lib/city3d/types';

export const DEFAULT_PALETTE: Palette = {
  key: 'default',
  slots: {
    wall: '#efe3cf',
    roof: '#b8553f',
    trim: '#f7f2ea',
    wood: '#8a5a3b',
    stone: '#a9a39a',
    glass: '#7fb2c9',
    door: '#5b3a2a',
    ground: '#a58f6b',
    grass: '#86b35a',
    water: '#4e9cc4',
    street: '#8b8680',
    plaza: '#cbbba0',
    leaf: '#5e9a45',
    trunk: '#6b4a33',
    metal: '#5d636b',
    cloth: '#d77a61',
  },
};
