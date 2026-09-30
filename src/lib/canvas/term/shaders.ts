// GLSL for the terminal pass: one quad per terminal body. The fragment shader
// finds the cell, fetches its texel (glyph slot, fg, bg, flags) and composes
// background, glyph from the atlas, underline/strike, selection and cursor.

export const TERM_VS = `#version 300 es
in vec2 a_pos;
uniform vec4 u_rect;      // body rect in device px: x, y, w, h (top-left origin)
uniform vec2 u_viewport;  // drawing buffer size in device px
out vec2 v_local;
void main() {
  vec2 p = u_rect.xy + a_pos * u_rect.zw;
  v_local = a_pos * u_rect.zw;
  gl_Position = vec4(p.x / u_viewport.x * 2.0 - 1.0, 1.0 - p.y / u_viewport.y * 2.0, 0.0, 1.0);
}
`;

export const TERM_FS = `#version 300 es
precision highp float;
precision highp int;
precision highp usampler2D;

uniform usampler2D u_cells;   // RGBA32UI, cols x rows
uniform usampler2D u_mini;    // R8UI, cols x rows
uniform sampler2D u_atlas;    // RGBA8 glyph grid
uniform sampler2D u_palette;  // 256 x 1 RGBA8
uniform int u_mode;           // 0 text, 1 minimap
uniform vec2 u_cell;          // cell size in device px
uniform ivec2 u_grid;         // cols, rows
uniform vec3 u_atlasCell;     // cw, ch, perRow
uniform vec2 u_atlasSize;
uniform vec4 u_defFg;
uniform vec4 u_defBg;
uniform vec4 u_cursorColor;
uniform vec4 u_selColor;
uniform vec4 u_matchColor;
uniform vec4 u_cursor;        // x, y, shape, visible (0/1)
uniform float u_focused;

in vec2 v_local;
out vec4 o;

vec4 unpackColor(uint c) {
  return vec4(float((c >> 24) & 255u), float((c >> 16) & 255u), float((c >> 8) & 255u), float(c & 255u)) / 255.0;
}

float band(float y, float y0, float h) {
  return step(y0, y) * step(y, y0 + h);
}

void outColor(vec4 c) {
  o = vec4(c.rgb * c.a, c.a);
}

void main() {
  vec2 cellf = v_local / u_cell;
  ivec2 cell = ivec2(floor(cellf));
  if (cell.x >= u_grid.x || cell.y >= u_grid.y || cell.x < 0 || cell.y < 0) {
    outColor(u_defBg);
    return;
  }
  vec2 f = fract(cellf);

  if (u_mode == 1) {
    uint idx = texelFetch(u_mini, cell, 0).r;
    vec4 col = u_defBg;
    if (idx != 0u) {
      vec4 c = idx == 255u ? u_defFg : texelFetch(u_palette, ivec2(int(idx), 0), 0);
      float inset = step(0.18, f.y) * step(f.y, 0.82);
      col = mix(u_defBg, vec4(c.rgb, 1.0), 0.8 * inset);
    }
    outColor(col);
    return;
  }

  uvec4 t = texelFetch(u_cells, cell, 0);
  vec4 fg = (t.g & 255u) == 0u ? u_defFg : unpackColor(t.g);
  vec4 bg = (t.b & 255u) == 0u ? u_defBg : unpackColor(t.b);
  uint fl = t.a;

  if ((fl & 32u) != 0u) bg = vec4(mix(bg.rgb, u_matchColor.rgb, u_matchColor.a), max(bg.a, u_matchColor.a));
  if ((fl & 16u) != 0u) bg = vec4(mix(bg.rgb, u_selColor.rgb, u_selColor.a), max(bg.a, u_selColor.a));

  bool isCursor = u_cursor.w > 0.5 && cell == ivec2(u_cursor.xy);
  int shape = int(u_cursor.z);
  bool hollow = isCursor && (shape == 3 || (shape == 0 && u_focused < 0.5));
  if (isCursor && shape == 0 && u_focused > 0.5) {
    bg = u_cursorColor;
    fg = vec4(u_defBg.rgb, 1.0);
  }

  vec4 col = bg;
  uint slot = t.r & 0xFFFFFFu;
  if (slot != 0u) {
    float perRow = u_atlasCell.z;
    float s = float(slot);
    vec2 origin = vec2(mod(s, perRow) * u_atlasCell.x, floor(s / perRow) * u_atlasCell.y);
    vec2 p = origin + clamp(f * u_atlasCell.xy, vec2(0.5), u_atlasCell.xy - 0.5);
    vec4 g = texture(u_atlas, p / u_atlasSize);
    if ((fl & 128u) != 0u) {
      col = vec4(mix(col.rgb, g.rgb, g.a), max(col.a, g.a));
    } else {
      col = vec4(mix(col.rgb, fg.rgb, g.a * fg.a), max(col.a, g.a * fg.a));
    }
  }

  // Decorations, in device pixels of this cell.
  float py = f.y * u_cell.y;
  float px = f.x * u_cell.x;
  float thick = max(1.0, floor(u_cell.y / 16.0));
  float base = u_cell.y - 2.0 * thick;
  uint ul = fl & 7u;
  float line = 0.0;
  if (ul == 1u) line = band(py, base, thick - 0.01);
  else if (ul == 2u) line = max(band(py, base, thick - 0.01), band(py, base - 2.0 * thick, thick - 0.01));
  else if (ul == 3u) {
    float yy = base - thick + sin(v_local.x / max(u_cell.x, 1.0) * 6.2831853) * thick;
    line = 1.0 - smoothstep(0.0, thick, abs(py - yy));
  } else if (ul == 4u) line = band(py, base, thick - 0.01) * step(0.5, fract(v_local.x / (thick * 4.0)));
  else if (ul == 5u) line = band(py, base, thick - 0.01) * step(0.35, fract(v_local.x / (u_cell.x)));
  if (line > 0.0) {
    uint ulc = fl >> 8;
    vec4 uc = ulc == 0u ? fg : vec4(float((ulc >> 16) & 255u), float((ulc >> 8) & 255u), float(ulc & 255u), 255.0) / 255.0;
    col = vec4(mix(col.rgb, uc.rgb, line * uc.a), max(col.a, line));
  }
  if ((fl & 8u) != 0u) {
    float st = band(py, floor(u_cell.y * 0.55), thick - 0.01);
    col = vec4(mix(col.rgb, fg.rgb, st), max(col.a, st));
  }

  if (isCursor) {
    float w = max(1.0, floor(u_cell.x / 8.0));
    float c = 0.0;
    if (hollow) c = max(max(step(px, w), step(u_cell.x - w, px)), max(step(py, w), step(u_cell.y - w, py)));
    else if (shape == 1) c = step(u_cell.y - max(2.0, w), py);
    else if (shape == 2) c = step(px, max(2.0, w));
    if (c > 0.0) col = vec4(mix(col.rgb, u_cursorColor.rgb, c), max(col.a, c));
  }

  outColor(col);
}
`;
