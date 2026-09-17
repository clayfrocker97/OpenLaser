/** Deterministic, native SVG material surfaces. Run with node ui/scripts/generate-material-art.mjs. */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root = process.env.MATERIAL_ART_ROOT || path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../static/materials');
const rand = (s) => { let h = [...s].reduce((a,c) => Math.imul(a ^ c.charCodeAt(0),16777619)>>>0,2166136261); return () => ((h = Math.imul(h,1664525)+1013904223>>>0)/4294967296); };
const woods = {
 'birch-plywood':['#e4c798','#b18955'], 'baltic-birch-plywood':['#edd6ac','#bd9967'], 'bamboo-plywood':['#c6a060','#876432'], 'wood-veneer':['#c38d56','#7c4926'], balsa:['#f2dfb8','#c3a779'], basswood:['#e8d2a8','#b49561'], maple:['#e3be85','#a67c46'], 'white-oak':['#c6ac7b','#826842'], 'red-oak':['#c68d63','#845239'], walnut:['#67442e','#302419'], cherry:['#b86d48','#6f3b27'], mahogany:['#8b4730','#4d261e'], pine:['#e3b774','#a27137'], cedar:['#bd7750','#703c2b']
};
const metals = {
 'mild-steel':['#77828a','#48535c'], 'hot-rolled-steel':['#4a545b','#242f38'], 'cold-rolled-steel':['#99a6af','#596972'], 'stainless-steel':['#b6c3c7','#70838b'], 'brushed-stainless':['#a0b1b9','#687b86'], 'mirror-stainless':['#d0dcdf','#69828e'], 'galvanized-steel':['#a4b5ba','#6e8992'], 'tool-steel':['#627681','#344b59'], aluminum:['#d0d9dc','#8f9fa5'], 'brushed-aluminum':['#bccbd1','#7b919d'], 'anodized-aluminum':['#6b899c','#314e65'], 'black-anodized-aluminum':['#384148','#151e25'], 'powder-coated-metal':['#395e61','#203b40'], 'painted-metal':['#9e4740','#682c2b'], brass:['#c6a456','#796126'], copper:['#c77f60','#754232'], bronze:['#a88751','#625335'], titanium:['#9596ac','#555e76'], nickel:['#afbab5','#737f79'], silver:['#d9e1e0','#8e9b9c'], gold:['#dbb559','#9a742a']
};
const fibers = { 'pet-felt':['#788a87','#526865'], 'wool-felt':['#9c8d80','#6c6056'], cotton:['#e5ddc8','#b2aa96'], linen:['#cabb9b','#918469'], denim:['#456c8a','#233f58'], canvas:['#c7af85','#8d7752'], 'polyester-fabric':['#739596','#456768'], silk:['#b298ae','#715b72'], 'vegetable-tanned-leather':['#bb774c','#7e452c'], suede:['#a08467','#6c513e'], paper:['#eee9da','#c5bcaa'], cardstock:['#b9c5b7','#8d9a89'], 'kraft-paper':['#bc9560','#826138'], 'corrugated-cardboard':['#b28b58','#80603a'], chipboard:['#a58f6d','#715f45'], 'bookbinding-board':['#8b9390','#616b68'], mdf:['#b09265','#7e653f'], hdf:['#9b7950','#6a4e31'], hardboard:['#826043','#533e2d'], cork:['#be9061','#795235'], 'laser-rubber':['#856b68','#584644'] };
const surfaces = { 'clear-acrylic':['#c8e5e6','#8cb8c0'], 'frosted-acrylic':['#d4e4e3','#a4c1c3'], 'white-acrylic':['#f2f1eb','#d6dcd8'], 'black-acrylic':['#27343e','#101c24'], 'colored-acrylic':['#168d9a','#075260'], 'translucent-acrylic':['#70b9b5','#348782'], 'fluorescent-acrylic':['#e88b38','#faca52'], 'mirror-acrylic':['#c3c6db','#7d8aab'], 'two-layer-engraving-acrylic':['#beaa78','#766749'], 'acetal-delrin':['#f1e8d5','#c7bda8'], glass:['#b7d7cf','#6ba99a'], 'mirror-glass':['#c5d9dc','#70989f'], slate:['#58666c','#303e44'], granite:['#9c9d99','#555b5b'], marble:['#e6e4da','#929e99'], 'ceramic-tile':['#becbb8','#8c9e86'], porcelain:['#e9e7e1','#bbc3c2'], terracotta:['#bd7358','#7e493b'], 'laser-marking-laminate':['#aaafaa','#65736d'] };
const ids=fs.readdirSync(process.env.MATERIAL_ART_SOURCE || root).filter(x=>x.endsWith('.webp')).map(x=>x.slice(0,-5)).sort();
const f=x=>x.toFixed(1);
for (const id of ids) {
 const r=rand(id), cfg=woods[id]||metals[id]||fibers[id]||surfaces[id]; if(!cfg)throw Error(id);
 const [base,dark]=cfg; let art='', defs=`<linearGradient id="base" x2="1" y2="1"><stop stop-color="${base}"/><stop offset="1" stop-color="${dark}"/></linearGradient><linearGradient id="light" x2=".7" y2="1"><stop stop-color="#fff" stop-opacity=".16"/><stop offset=".5" stop-color="#fff" stop-opacity="0"/><stop offset="1" stop-color="#000" stop-opacity=".07"/></linearGradient>`;
 const p=(d,c,o,w=1)=>`<path d="${d}" fill="none" stroke="${c}" stroke-opacity="${o}" stroke-width="${w}"/>`;
 const dots=(n,scale=2)=>Array.from({length:n},()=>`<ellipse cx="${f(r()*800)}" cy="${f(r()*600)}" rx="${f(.4+r()*scale)}" ry="${f(.3+r()*scale*.55)}" fill="${r()>.35?dark:'#fff'}" opacity="${f(.1+r()*.16)}"/>`).join('');
 if(woods[id]) {
  art=`<rect width="800" height="600" fill="${base}"/>`;
  for(let i=0;i<65;i++){const x=i*13-20, bend=(r()-.5)*95;art+=p(`M${x} -30 C${f(x+bend)} 130 ${f(x-bend)} 290 ${f(x+bend*.4)} 630`,dark,f(.1+r()*.2),f(.5+r()*2.4));}
  const knotX=210+r()*380, knotY=180+r()*220;
  for(let i=0;i<16;i++){let rx=8+i*7, ry=15+i*20;art+=p(`M${f(knotX)} ${f(knotY-ry)} C${f(knotX+rx*1.8)} ${f(knotY-ry*.3)} ${f(knotX+rx)} ${f(knotY+ry*.6)} ${f(knotX)} ${f(knotY+ry)} C${f(knotX-rx)} ${f(knotY+ry*.4)} ${f(knotX-rx*1.5)} ${f(knotY-ry*.4)} ${f(knotX)} ${f(knotY-ry)}`,dark,.14,1.4);}
  if(id==='bamboo-plywood')for(let x=50;x<800;x+=110){art+=p(`M${x} 0V600`,dark,.38,3);art+=p(`M${x} ${80+x*.45}h110`,dark,.36,4);}
 } else if(metals[id]) {
  defs+=`<linearGradient id="metal" x1="0" y1="0" x2="1" y2=".55"><stop stop-color="${dark}"/><stop offset=".35" stop-color="${base}"/><stop offset=".57" stop-color="${base}"/><stop offset="1" stop-color="${dark}"/></linearGradient>`;
  art='<rect width="800" height="600" fill="url(#metal)"/>';
  if(id==='galvanized-steel')for(let i=0;i<125;i++){const x=r()*850-25,y=r()*650-25,s=15+r()*53;art+=`<path d="M${f(x-s)} ${f(y)}l${f(s*.6)} ${f(-s*.8)} ${f(s*1.1)} ${f(s*.1)} ${f(s*.5)} ${f(s*.9)} ${f(-s*.8)} ${f(s*.7)}Z" fill="${r()>.5?'#e2eeed':dark}" opacity="${f(.1+r()*.24)}"/>`;}
  else if(id==='powder-coated-metal'||id==='painted-metal')art+=dots(600,1.8);
  else for(let i=0;i<185;i++){let y=r()*600,x=r()*100;art+=p(`M${f(x-100)} ${f(y)}h${f(500+r()*400)}`,r()>.5?'#fff':dark,f(.035+r()*.075),f(.3+r()*.7));}
  if(id.includes('mirror')||id==='silver')art+='<path d="M-100 600L360 0h95L-5 600Z" fill="#fff" opacity=".2"/>';
 } else if(fibers[id]) {
  art=`<rect width="800" height="600" fill="${base}"/>`;
  if(['cotton','linen','denim','canvas','polyester-fabric'].includes(id)) {
   defs+=`<pattern id="weave" width="16" height="16" patternUnits="userSpaceOnUse"><path d="M0 3H16M0 11H16" stroke="${dark}" stroke-width="3" opacity=".2"/><path d="M3 0V16M11 0V16" stroke="#fff" stroke-width="2" opacity=".18"/><path d="M2 2h4M10 10h4" stroke="${dark}" opacity=".25" stroke-width="2"/></pattern>`;
   art+='<rect width="800" height="600" fill="url(#weave)"/>';
   if(id==='denim')for(let i=-600;i<800;i+=9)art+=p(`M${i} 0l600 600`,'#c4d7de',.14,2);
  } else if(id==='silk'){for(let i=-300;i<900;i+=80)art+=p(`M${i} -30Q${i+160} 260 ${i-40} 650`,'#fff',.09,30);}
  else if(id==='corrugated-cardboard'){for(let x=0;x<800;x+=28)art+=p(`M${x} 0V600`,dark,.13,9);art+=dots(240,2);}
  else if(id==='cork'||id==='chipboard'){for(let i=0;i<430;i++){const x=r()*800,y=r()*600,s=3+r()*14;art+=`<path d="M${f(x)} ${f(y)}l${f(s)} ${f(-s*.3)} ${f(s*.4)} ${f(s*.8)} ${f(-s)} ${f(s*.5)}Z" fill="${r()>.35?dark:'#eee0b4'}" opacity="${f(.12+r()*.28)}"/>`;}}
  else {art+=dots(420,2);for(let i=0;i<330;i++){let x=r()*800,y=r()*600;art+=p(`M${f(x)} ${f(y)}l${f(r()*10-5)} ${f(r()*8-4)}`,r()>.4?dark:'#fff',.16,.7);}}
 } else {
  art='<rect width="800" height="600" fill="url(#base)"/>';
  if(id==='granite'){art+=dots(1050,7);}
  else if(id==='marble'||id==='slate'){for(let i=0;i<20;i++){let y=r()*600;art+=p(`M-50 ${f(y)}l170 -${f(30+r()*90)} 140 ${f(r()*60)} 190 -${f(r()*80)} 150 ${f(r()*60)} 260 -80`,dark,f(.08+r()*.22),f(.5+r()*3));}}
  else if(['terracotta','ceramic-tile','porcelain','frosted-acrylic','acetal-delrin'].includes(id))art+=dots(520,1.8);
  else {
   defs+='<linearGradient id="reflection"><stop stop-color="#fff" stop-opacity="0"/><stop offset=".5" stop-color="#fff" stop-opacity=".22"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></linearGradient>';
   art+='<path d="M-300 600L250 0h260L-40 600Z" fill="url(#reflection)"/><path d="M440 600L990 0h80L520 600Z" fill="#fff" opacity=".1"/>';
   if(id==='clear-acrylic'||id==='glass'||id==='translucent-acrylic')art+='<path d="M0 445Q330 360 800 435" fill="none" stroke="#fff" stroke-width="2" opacity=".19"/>';
   if(id==='two-layer-engraving-acrylic'||id==='laser-marking-laminate')for(let i=0;i<100;i++)art+=p(`M0 ${i*6}H800`,dark,.06,.5);
  }
 }
 const svg=`<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600" viewBox="0 0 800 600"><defs>${defs}</defs>${art}<rect width="800" height="600" fill="url(#light)"/></svg>\n`;
 fs.writeFileSync(path.join(root,id+'.svg'),svg);
}
console.log(`Generated ${ids.length} native SVG material surfaces.`);
