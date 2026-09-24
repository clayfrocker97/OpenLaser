import type { LaserMode, RecipeView } from '../api';

// The material cards: the illustrations under ui/static/materials, one
// per material, with the names the picker offers. Generated from the
// card collection's manifest.

export interface MaterialCard { id: string; name: string; category: string }

export const CARDS: MaterialCard[] = [
  { id: 'mild-steel', name: "Mild steel", category: "Metals" },
  { id: 'hot-rolled-steel', name: "Hot-rolled steel", category: "Metals" },
  { id: 'cold-rolled-steel', name: "Cold-rolled steel", category: "Metals" },
  { id: 'stainless-steel', name: "Stainless steel", category: "Metals" },
  { id: 'brushed-stainless', name: "Brushed stainless", category: "Metals" },
  { id: 'mirror-stainless', name: "Mirror stainless", category: "Metals" },
  { id: 'galvanized-steel', name: "Galvanized steel", category: "Metals" },
  { id: 'tool-steel', name: "Tool steel", category: "Metals" },
  { id: 'aluminum', name: "Aluminum", category: "Metals" },
  { id: 'brushed-aluminum', name: "Brushed aluminum", category: "Metals" },
  { id: 'anodized-aluminum', name: "Anodized aluminum", category: "Metals" },
  { id: 'black-anodized-aluminum', name: "Black anodized aluminum", category: "Metals" },
  { id: 'powder-coated-metal', name: "Powder-coated metal", category: "Metals" },
  { id: 'painted-metal', name: "Painted metal", category: "Metals" },
  { id: 'brass', name: "Brass", category: "Metals" },
  { id: 'copper', name: "Copper", category: "Metals" },
  { id: 'bronze', name: "Bronze", category: "Metals" },
  { id: 'titanium', name: "Titanium", category: "Metals" },
  { id: 'nickel', name: "Nickel", category: "Metals" },
  { id: 'silver', name: "Silver", category: "Metals" },
  { id: 'gold', name: "Gold", category: "Metals" },
  { id: 'birch-plywood', name: "Birch plywood", category: "Wood & board" },
  { id: 'baltic-birch-plywood', name: "Baltic birch plywood", category: "Wood & board" },
  { id: 'bamboo-plywood', name: "Bamboo plywood", category: "Wood & board" },
  { id: 'mdf', name: "MDF", category: "Wood & board" },
  { id: 'hdf', name: "HDF", category: "Wood & board" },
  { id: 'hardboard', name: "Hardboard", category: "Wood & board" },
  { id: 'wood-veneer', name: "Wood veneer", category: "Wood & board" },
  { id: 'balsa', name: "Balsa", category: "Wood & board" },
  { id: 'basswood', name: "Basswood", category: "Wood & board" },
  { id: 'maple', name: "Maple", category: "Wood & board" },
  { id: 'white-oak', name: "White oak", category: "Wood & board" },
  { id: 'red-oak', name: "Red oak", category: "Wood & board" },
  { id: 'walnut', name: "Walnut", category: "Wood & board" },
  { id: 'cherry', name: "Cherry", category: "Wood & board" },
  { id: 'mahogany', name: "Mahogany", category: "Wood & board" },
  { id: 'pine', name: "Pine", category: "Wood & board" },
  { id: 'cedar', name: "Cedar", category: "Wood & board" },
  { id: 'cork', name: "Cork", category: "Wood & board" },
  { id: 'clear-acrylic', name: "Clear acrylic", category: "Acrylic & polymers" },
  { id: 'frosted-acrylic', name: "Frosted acrylic", category: "Acrylic & polymers" },
  { id: 'white-acrylic', name: "White acrylic", category: "Acrylic & polymers" },
  { id: 'black-acrylic', name: "Black acrylic", category: "Acrylic & polymers" },
  { id: 'colored-acrylic', name: "Colored acrylic", category: "Acrylic & polymers" },
  { id: 'translucent-acrylic', name: "Translucent acrylic", category: "Acrylic & polymers" },
  { id: 'fluorescent-acrylic', name: "Fluorescent acrylic", category: "Acrylic & polymers" },
  { id: 'mirror-acrylic', name: "Mirror acrylic", category: "Acrylic & polymers" },
  { id: 'two-layer-engraving-acrylic', name: "Two-layer engraving acrylic", category: "Acrylic & polymers" },
  { id: 'acetal-delrin', name: "Acetal / Delrin", category: "Acrylic & polymers" },
  { id: 'pet-felt', name: "PET felt", category: "Textiles & soft goods" },
  { id: 'wool-felt', name: "Wool felt", category: "Textiles & soft goods" },
  { id: 'cotton', name: "Cotton", category: "Textiles & soft goods" },
  { id: 'linen', name: "Linen", category: "Textiles & soft goods" },
  { id: 'denim', name: "Denim", category: "Textiles & soft goods" },
  { id: 'canvas', name: "Canvas", category: "Textiles & soft goods" },
  { id: 'polyester-fabric', name: "Polyester fabric", category: "Textiles & soft goods" },
  { id: 'silk', name: "Silk", category: "Textiles & soft goods" },
  { id: 'vegetable-tanned-leather', name: "Vegetable-tanned leather", category: "Textiles & soft goods" },
  { id: 'suede', name: "Suede", category: "Textiles & soft goods" },
  { id: 'paper', name: "Paper", category: "Paper & packaging" },
  { id: 'cardstock', name: "Cardstock", category: "Paper & packaging" },
  { id: 'kraft-paper', name: "Kraft paper", category: "Paper & packaging" },
  { id: 'corrugated-cardboard', name: "Corrugated cardboard", category: "Paper & packaging" },
  { id: 'chipboard', name: "Chipboard", category: "Paper & packaging" },
  { id: 'bookbinding-board', name: "Bookbinding board", category: "Paper & packaging" },
  { id: 'glass', name: "Glass", category: "Stone & glass" },
  { id: 'mirror-glass', name: "Mirror glass", category: "Stone & glass" },
  { id: 'slate', name: "Slate", category: "Stone & glass" },
  { id: 'granite', name: "Granite", category: "Stone & glass" },
  { id: 'marble', name: "Marble", category: "Stone & glass" },
  { id: 'ceramic-tile', name: "Ceramic tile", category: "Stone & glass" },
  { id: 'porcelain', name: "Porcelain", category: "Stone & glass" },
  { id: 'terracotta', name: "Terracotta", category: "Stone & glass" },
  { id: 'laser-rubber', name: "Laser rubber", category: "Specialty materials" },
  { id: 'laser-marking-laminate', name: "Laser marking laminate", category: "Specialty materials" },
];

/** The card's picture. */
export const cardUrl = (id: string): string => `/materials/${id}.svg`;

/** A material's picture: its sample cut photo, else its card. */
export function pictureOf(name: string, photo: string | null | undefined): string | null {
  if (photo) return `/api/photos/${photo}`;
  const card = cardFor(name);
  return card ? cardUrl(card.id) : null;
}

/** The card a material name means: its own name, a vendor's short name, or a family word. */
export function cardFor(name: string): MaterialCard | null {
  const key = name.trim().toLowerCase();
  const exact = CARDS.find((c) => c.name.toLowerCase() === key || c.id === key);
  if (exact) return exact;
  const alias: Array<[RegExp, string]> = [
    [/^(ss|stainless)/, 'stainless-steel'], [/^(cs|carbon|mild|steel|ms\b)/, 'mild-steel'], [/^(al\b|alu)/, 'aluminum'],
    [/^(cu\b|copper)/, 'copper'], [/brass/, 'brass'], [/bronze/, 'bronze'], [/titanium|^ti\b/, 'titanium'], [/galvan/, 'galvanized-steel'],
    [/acrylic|pmma|plexi/, 'clear-acrylic'], [/delrin|acetal|pom/, 'acetal-delrin'], [/rubber/, 'laser-rubber'],
    [/basswood/, 'basswood'], [/birch|ply/, 'birch-plywood'], [/mdf/, 'mdf'], [/hdf/, 'hdf'], [/oak/, 'red-oak'], [/walnut/, 'walnut'],
    [/maple/, 'maple'], [/cherry/, 'cherry'], [/pine/, 'pine'], [/balsa/, 'balsa'], [/bamboo/, 'bamboo-plywood'], [/cork/, 'cork'],
    [/veneer/, 'wood-veneer'], [/wood/, 'basswood'],
    [/cardboard|corrugat/, 'corrugated-cardboard'], [/cardstock|card\b/, 'cardstock'], [/paper/, 'paper'], [/leather/, 'vegetable-tanned-leather'],
    [/felt/, 'wool-felt'], [/denim/, 'denim'], [/cotton/, 'cotton'], [/fabric|textile|cloth/, 'polyester-fabric'],
    [/glass/, 'glass'], [/tile|ceramic/, 'ceramic-tile'], [/slate/, 'slate'], [/marble/, 'marble'], [/granite/, 'granite'],
  ];
  const id = alias.find(([pattern]) => pattern.test(key))?.[1];
  return id ? CARDS.find((c) => c.id === id) ?? null : null;
}

/** Library groups use laser plus material, just as recipe identity does. */
export const materialKey = (recipe: Pick<RecipeView, 'laser' | 'name'>): string => JSON.stringify([recipe.laser, recipe.name]);
export type Material = { key: string; name: string; laser: LaserMode; recipes: RecipeView[]; photo: string | null };
export function materialsOf(recipes: readonly RecipeView[]): Material[] {
  const groups = new Map<string, Material>();
  for (const recipe of recipes) {
    const key = materialKey(recipe);
    const group = groups.get(key) ?? { key, name: recipe.name, laser: recipe.laser, recipes: [], photo: null };
    group.recipes.push(recipe);
    group.photo ??= recipe.photo;
    groups.set(key, group);
  }
  for (const group of groups.values()) group.recipes.sort((a, b) => a.thickness_mm - b.thickness_mm || a.gas.localeCompare(b.gas));
  return [...groups.values()].sort((a, b) => Number(b.recipes.some((r) => r.favourite)) - Number(a.recipes.some((r) => r.favourite)) || a.name.localeCompare(b.name) || a.laser.localeCompare(b.laser));
}
