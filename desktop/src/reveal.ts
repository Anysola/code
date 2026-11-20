/** Replies being written, by stream key: their text and how much of it is shown. The stored row
 * that replaces a live one starts from there (`transcript.tsx`); a stopped turn clears it, so
 * what was written shows at once (`store.ts`). */
export const revealing = new Map<string, { text: string; shown: number }>()
