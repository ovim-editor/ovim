"""Draws the Ovim application icon as SVG: an O and Vim's diamond on a
macOS-style tile with margins and a drop shadow.

Run: python3 icons/app-icon.py icons/icon.svg
"""
import math
def squircle(cx, cy, rx, ry, n, steps=240):
    pts=[]
    for i in range(steps):
        t=2*math.pi*i/steps
        c,s=math.cos(t),math.sin(t)
        x=cx+rx*math.copysign(abs(c)**(2/n),c)
        y=cy+ry*math.copysign(abs(s)**(2/n),s)
        pts.append(f"{x:.2f} {y:.2f}")
    return "M"+" L".join(pts)+"Z"

body = squircle(512,512,412,412,5.0)
ring_outer = squircle(512,492,236,236,2.25)
ring_inner = squircle(512,492,128,128,2.25)
ring = ring_outer + " " + ring_inner

# code lines behind the mark: (indent, width) in editor columns
lines=[(0,9),(1,14),(2,11),(2,17),(1,6),(0,2),(0,12),(1,16),(2,9),(1,13),(0,3),(0,10),(1,15)]
code=[]
y0=214; step=48
for i,(indent,w) in enumerate(lines):
    y=y0+i*step
    if y>830: break
    x=230+indent*44
    code.append(f'<rect x="{x}" y="{y}" width="{w*28}" height="16" rx="8"/>')
    code.append(f'<rect x="168" y="{y}" width="30" height="16" rx="8" opacity=".55"/>')

# faceted gem (vim's diamond), center and radius
gx,gy,gr=700,686,96
tr=gr*0.42
top=(gx,gy-gr); right=(gx+gr,gy); bot=(gx,gy+gr); left=(gx-gr,gy)
ttop=(gx,gy-tr); tright=(gx+tr,gy); tbot=(gx,gy+tr); tleft=(gx-tr,gy)
P=lambda *ps: "M"+" L".join(f"{x:.1f} {y:.1f}" for x,y in ps)+"Z"
gem_outline=P(top,right,bot,left)
facets=[
 (P(top,right,tright,ttop),'#d9f5a8'),
 (P(right,bot,tbot,tright),'#86b850'),
 (P(bot,left,tleft,tbot),'#a3d36a'),
 (P(left,top,ttop,tleft),'#eefbd0'),
 (P(ttop,tright,tbot,tleft),'url(#gemTable)'),
]
facets_svg="\n    ".join(f'<path d="{d}" fill="{f}"/>' for d,f in facets)

svg=f'''<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <defs>
    <linearGradient id="body" x1="250" y1="100" x2="780" y2="924" gradientUnits="userSpaceOnUse">
      <stop stop-color="#1f2750"/>
      <stop offset=".55" stop-color="#121628"/>
      <stop offset="1" stop-color="#07080f"/>
    </linearGradient>
    <radialGradient id="halo" cx="512" cy="492" r="400" gradientUnits="userSpaceOnUse">
      <stop stop-color="#7f9cff" stop-opacity=".38"/>
      <stop offset=".55" stop-color="#6d5bd8" stop-opacity=".12"/>
      <stop offset="1" stop-color="#6d5bd8" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="sheen" x1="0" y1="100" x2="0" y2="560" gradientUnits="userSpaceOnUse">
      <stop stop-color="#fff" stop-opacity=".07"/>
      <stop offset="1" stop-color="#fff" stop-opacity="0"/>
    </linearGradient>
    <linearGradient id="edge" x1="0" y1="100" x2="0" y2="924" gradientUnits="userSpaceOnUse">
      <stop stop-color="#fff" stop-opacity=".22"/>
      <stop offset=".5" stop-color="#fff" stop-opacity=".05"/>
      <stop offset="1" stop-color="#fff" stop-opacity=".1"/>
    </linearGradient>
    <linearGradient id="ring" x1="300" y1="230" x2="730" y2="760" gradientUnits="userSpaceOnUse">
      <stop stop-color="#bcd0ff"/>
      <stop offset=".38" stop-color="#82aaff"/>
      <stop offset=".78" stop-color="#9d86ff"/>
      <stop offset="1" stop-color="#c58cff"/>
    </linearGradient>
    <linearGradient id="ringBevel" x1="0" y1="230" x2="0" y2="754" gradientUnits="userSpaceOnUse">
      <stop stop-color="#fff" stop-opacity=".38"/>
      <stop offset=".22" stop-color="#fff" stop-opacity=".08"/>
      <stop offset=".6" stop-color="#000" stop-opacity="0"/>
      <stop offset="1" stop-color="#1a0b3d" stop-opacity=".35"/>
    </linearGradient>
    <linearGradient id="gemTable" x1="{gx-tr}" y1="{gy-tr}" x2="{gx+tr}" y2="{gy+tr}" gradientUnits="userSpaceOnUse">
      <stop stop-color="#f4ffe0"/>
      <stop offset="1" stop-color="#c3e88d"/>
    </linearGradient>
    <clipPath id="bodyClip"><path d="{body}"/></clipPath>
    <clipPath id="ringClip"><path d="{ring}" clip-rule="evenodd"/></clipPath>
    <filter id="shadow" x="-20%" y="-20%" width="140%" height="140%">
      <feGaussianBlur stdDeviation="14"/>
    </filter>
    <filter id="glow" x="-50%" y="-50%" width="200%" height="200%">
      <feGaussianBlur stdDeviation="26"/>
    </filter>
    <filter id="innerShadow" x="-10%" y="-10%" width="120%" height="120%">
      <feGaussianBlur stdDeviation="6"/>
    </filter>
  </defs>

  <!-- Drop shadow under the tile, as macOS icons have -->
  <path d="{body}" transform="translate(0 14)" fill="#000" opacity=".42" filter="url(#shadow)"/>

  <!-- Tile -->
  <path d="{body}" fill="url(#body)"/>
  <g clip-path="url(#bodyClip)">
    <rect width="1024" height="1024" fill="url(#halo)"/>
    <g fill="#c8d3f5" opacity=".07">
      {"".join(code)}
    </g>
    <rect width="1024" height="1024" fill="url(#sheen)"/>
  </g>
  <path d="{body}" fill="none" stroke="url(#edge)" stroke-width="3"/>

  <!-- The O -->
  <path d="{ring}" fill-rule="evenodd" fill="#7f8cff" opacity=".75" filter="url(#glow)"/>
  <path d="{ring}" fill-rule="evenodd" fill="url(#ring)"/>
  <g clip-path="url(#ringClip)">
    <rect x="200" y="200" width="624" height="600" fill="url(#ringBevel)"/>
    <path d="{ring_inner}" fill="none" stroke="#120a33" stroke-opacity=".4" stroke-width="12" filter="url(#innerShadow)"/>
  </g>
  <path d="{ring}" fill-rule="evenodd" fill="none" stroke="#fff" stroke-opacity=".18" stroke-width="2"/>

  <!-- Vim's diamond, cut as a gem -->
  <path d="{gem_outline}" fill="#c3e88d" opacity=".55" filter="url(#glow)"/>
  <path d="{gem_outline}" fill="none" stroke="#0b0e1c" stroke-width="20" stroke-linejoin="round"/>
  <g>
    {facets_svg}
  </g>
  <path d="{gem_outline}" fill="none" stroke="#fff" stroke-opacity=".35" stroke-width="2" stroke-linejoin="round"/>
</svg>
'''
import sys
open(sys.argv[1] if len(sys.argv) > 1 else 'icon.svg','w').write(svg)
