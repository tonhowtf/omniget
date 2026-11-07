export const PET_CORNERS = ["top-left", "top-right", "bottom-left", "bottom-right"] as const;

export type PetCorner = (typeof PET_CORNERS)[number];

export const CORNER_POINTS: Record<PetCorner, { x: number; y: number }> = {
  "top-left": { x: 17, y: 15 },
  "top-right": { x: 63, y: 15 },
  "bottom-left": { x: 17, y: 35 },
  "bottom-right": { x: 63, y: 35 },
};

export function petCornerOrDefault(value: string | null | undefined): PetCorner {
  return (PET_CORNERS as readonly string[]).includes(value ?? "") ? (value as PetCorner) : "bottom-right";
}
