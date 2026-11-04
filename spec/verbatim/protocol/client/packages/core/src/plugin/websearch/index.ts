import { WebSearchAnysola } from "./anysola.js"

// Anysola Code: Anysola's gateway is the only web search provider; the third-party ones (Exa, Firecrawl, Parallel,
// Tavily, TinyFish) are removed.
export const WebSearchPlugins = [WebSearchAnysola.Plugin] as const
