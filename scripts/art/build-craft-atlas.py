#!/usr/bin/env python3
"""Prepare generated artwork for the existing atlas packer; no generation calls.
Requires Pillow. Original source art, house geometry and collision metadata are preserved; farm props are additive.
"""
import json
import shutil
from pathlib import Path
from PIL import Image, ImageOps
from farm_scene import prepare, generate

ROOT = Path(__file__).resolve().parents[2]
RAW = ROOT / 'art/world/craft-v1'
BASE = ROOT / 'art/world/casa-v1-src'
OUT = ROOT / 'art/world/craft-v1-src'
KEYS = ['floor/moss', 'floor/cobble', 'object/door', 'object/workbench',
        'object/chair', 'object/chest', 'object/bookshelf']


def main():
    shutil.copytree(BASE, OUT, dirs_exist_ok=True)
    for key in KEYS:
        original = Image.open(BASE / f'{key}.png').convert('RGBA')
        source_path = RAW / ('farm/workbench.png' if key == 'object/workbench' else f'{key}.png')
        source = Image.open(source_path).convert('RGBA')
        w, h = original.size
        if key.startswith('floor/'):
            # Map the full square texture into the renderer's 64x32 diamond.
            # Keeping its existing coverage mask avoids cracks between cells.
            source = source.convert('RGB').resize((256, 256), Image.Resampling.LANCZOS)
            tile = source.transform((w, h), Image.Transform.AFFINE,
                                    (256/w, 256/h, -128, -256/w, 256/h, 128),
                                    Image.Resampling.BICUBIC).convert('RGBA')
            tile.putalpha(original.getchannel('A'))
        else:
            # Trim only the transparent margin. Preserve generated alpha.
            box = source.getchannel('A').point(lambda a: 255 if a >= 32 else 0).getbbox()
            if box is None:
                raise ValueError(f'Empty generated sprite: {key}')
            sprite = ImageOps.contain(source.crop(box), (w, h), Image.Resampling.LANCZOS)
            tile = Image.new('RGBA', (w, h))
            tile.alpha_composite(sprite, ((w-sprite.width)//2, h-sprite.height))
        tile.save(OUT / f'{key}.png')
    house = json.loads((ROOT / 'static/world/house-v1.json').read_text())
    metadata = prepare(RAW, OUT)
    yard = generate(house, metadata)
    (ROOT / 'static/world/yard-v1.json').write_text(json.dumps(yard, indent=2) + '\n')
    print(f'Prepared {len(KEYS)} replacements and 5 farm props; preserved original house contracts.')


if __name__ == '__main__':
    main()
