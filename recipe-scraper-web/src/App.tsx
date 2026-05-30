import { createSignal, createResource, Show, createEffect } from "solid-js";

interface Recipe {
  url: string;
  title: string;
  total_time: number;
  ingredients: string[];
  instructions: string[];
  image: string;
}

interface SearchHit {
  recipe: Recipe;
  score: number;
}

interface SearchResults {
  hits: SearchHit[];
  total: number;
  offset: number;
  limit: number;
}

async function searchRecipes(
  q: string,
  offset: number,
): Promise<SearchResults | undefined> {
  if (!q.trim()) return undefined;
  const res = await fetch(
    `/api/recipes/search?q=${encodeURIComponent(q)}&limit=20&offset=${offset}`,
  );
  if (!res.ok) return undefined;
  return res.json();
}

const COLORS = {
  bg: "#15101D",
  surface: "#1E1524",
  input: "#291F38",
  border: "#0D0B12",
  borderBtn: "#100C16",
  accent: "#D50B0B",
  accentSecondary: "#C867B9",
  textMuted: "#666666",
  textInfo: "#555555",
  textHeader: "#888888",
  textPlaceholder: "#999999",
} as const;

const FONTS = {
  display: "'Zen Dots'",
  title: "'Michroma'",
  mono: "'Fira Code'",
  heading: "'Inter Tight'",
  body: "'Coda'",
} as const;

const cardStyle = `background: ${COLORS.surface}; border: 1px solid ${COLORS.border}; border-radius: 8px; box-shadow: 4px 4px 4px rgba(0,0,0,0.2);`;
const btnStyle = `background: ${COLORS.surface}; border: 1px solid ${COLORS.borderBtn}; border-radius: 4px; box-shadow: 4px 4px 4px rgba(0,0,0,0.2); cursor: pointer; font-family: ${FONTS.title}; font-size: 10px; color: ${COLORS.accent}; padding: 6px 14px;`;
const btnDisabled = `${btnStyle} opacity: 0.5; cursor: default;`;

const App = () => {
  const [query, setQuery] = createSignal("");
  const [offset, setOffset] = createSignal(0);

  const [results] = createResource(
    () => {
      const q = query().trim();
      if (!q) return undefined;
      return `${q}:${offset()}`;
    },
    async (key) => {
      const [q, offsetStr] = key.split(":");
      return searchRecipes(q, Number(offsetStr));
    },
  );

  const onInput = (e: Event) => {
    setQuery((e.target as HTMLInputElement).value);
    setOffset(0);
  };

  const goToPage = (page: number) => {
    setOffset((page - 1) * 20);
  };

  const [selected, setSelected] = createSignal<Recipe | null>(null);
  const [animatingRecipe, setAnimatingRecipe] = createSignal<Recipe | null>(
    null,
  );
  const [panelPhase, setPanelPhase] = createSignal<
    "closed" | "open" | "closing"
  >("closed");

  createEffect(() => {
    const recipe = selected();
    if (recipe) {
      setAnimatingRecipe(recipe);
      setPanelPhase("open");
    } else if (panelPhase() === "open") {
      setPanelPhase("closing");
      setTimeout(() => {
        setAnimatingRecipe(null);
        setPanelPhase("closed");
      }, 200);
    }
  });

  const data = () => results();
  const hits = () => data()?.hits ?? [];
  const total = () => data()?.total ?? 0;
  const pageCount = () => Math.ceil(total() / 20) || 1;
  const currentPage = () => Math.floor(offset() / 20) + 1;

  const shouldCenter = () => query().trim() === "";

  const panelOpen = () => panelPhase() === "open";
  const panelRevealed = () => panelPhase() !== "closed";
  const isClosing = () => panelPhase() === "closing";

  return (
    <div
      style={`max-width: 640px; margin: 0 auto; padding: 2rem 1rem; background: ${COLORS.bg}; min-height: 100vh; box-sizing: border-box; display: flex; flex-direction: column;`}
    >
      <div
        style={`flex-shrink: 0; height: ${shouldCenter() ? "calc(50vh - 80px)" : "0px"}; transition: height 0.35s ease; overflow: hidden;`}
      />

      <h1
        style={`font-family: ${FONTS.display}; font-size: 24px; font-weight: 400; color: ${COLORS.accent}; margin: 0 0 16px; flex-shrink: 0;`}
      >
        Recipe Search
      </h1>

      <input
        type="text"
        placeholder="Search recipes..."
        value={query()}
        onInput={onInput}
        style={`width: 100%; padding: 12px; font-family: ${FONTS.mono}; font-size: 16px; background: ${COLORS.input}; border: 1px solid ${COLORS.border}; border-radius: 8px; box-sizing: border-box; color: #fff; outline: none; box-shadow: 4px 4px 4px rgba(0,0,0,0.2); flex-shrink: 0;`}
      />

      <Show when={hits().length > 0 && query().trim() !== ""}>
        <p
          style={`font-family: ${FONTS.heading}; font-weight: 600; font-size: 14px; color: ${COLORS.textHeader}; margin: 16px 0 4px;`}
        >
          {total()} recipes found
        </p>

        <div style="display: flex; flex-direction: column; gap: 16px;">
          {hits().map((hit) => (
            <div
              onClick={() => setSelected(hit.recipe)}
              style={`${cardStyle} display: flex; gap: 16px; padding: 0; cursor: pointer;`}
            >
              {hit.recipe.image && (
                <img
                  src={hit.recipe.image}
                  alt={hit.recipe.title}
                  style="width: 80px; height: 80px; object-fit: cover; border-radius: 4px; flex-shrink: 0;"
                />
              )}
              <div style="padding: 16px 0;">
                <span
                  style={`font-family: ${FONTS.title}; font-size: 16px; font-weight: 400; color: ${COLORS.accentSecondary};`}
                >
                  {hit.recipe.title}
                </span>
                {hit.recipe.total_time > 0 && (
                  <p
                    style={`margin: 8px 0 0; font-family: ${FONTS.mono}; font-size: 13.6px; color: ${COLORS.textMuted};`}
                  >
                    {hit.recipe.total_time} min
                  </p>
                )}
              </div>
            </div>
          ))}
        </div>

        <div
          style={`display: flex; align-items: center; justify-content: center; gap: 16px; margin-top: 16px;`}
        >
          <button
            onClick={() => goToPage(currentPage() - 1)}
            disabled={currentPage() <= 1}
            style={currentPage() <= 1 ? btnDisabled : btnStyle}
          >
            ◀ Prev
          </button>
          <span
            style={`font-family: ${FONTS.mono}; font-size: 14.4px; color: ${COLORS.textInfo};`}
          >
            Page {currentPage()} of {pageCount()} ({total()} results)
          </span>
          <button
            onClick={() => goToPage(currentPage() + 1)}
            disabled={currentPage() >= pageCount()}
            style={currentPage() >= pageCount() ? btnDisabled : btnStyle}
          >
            Next ▶
          </button>
        </div>
      </Show>

      <Show when={query().trim() && hits().length === 0 && !results.loading}>
        <div
          style={`display: flex; flex-direction: column; align-items: center; justify-content: center; margin-top: 64px;`}
        >
          <p
            style={`font-family: ${FONTS.title}; font-size: 18px; color: ${COLORS.accentSecondary}; margin: 0 0 8px;`}
          >
            No recipes found
          </p>
          <p
            style={`font-family: ${FONTS.mono}; font-size: 14px; color: ${COLORS.textMuted}; margin: 0;`}
          >
            Try adjusting your search term
          </p>
        </div>
      </Show>

      <style>{`
        @keyframes panel-slide { from { transform: translateX(100%); } to { transform: translateX(0); } }
        @keyframes panel-fade { from { opacity: 0; } to { opacity: 1; } }
      `}</style>

      <Show when={panelRevealed()}>
        <div
          onClick={() => setSelected(null)}
          style={`position: fixed; inset: 0; background: rgba(0,0,0,0.5); z-index: 998; animation: panel-fade 0.15s ease; opacity: ${isClosing() ? 0 : 1}; transition: opacity 0.2s ease;`}
        />
      </Show>

      <Show when={animatingRecipe()} keyed>
        {(recipe) => (
          <div
            style={`position: fixed; top: 0; right: 0; bottom: 0; width: min(480px, 100vw); background: ${COLORS.bg}; z-index: 999; overflow-y: auto; box-shadow: -4px 0 12px rgba(0,0,0,0.2); padding: 24px; box-sizing: border-box; animation: panel-slide 0.2s ease; transform: ${isClosing() ? "translateX(100%)" : "translateX(0)"}; transition: transform 0.2s ease;`}
          >
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px;">
              <h2
                style={`margin: 0; font-family: ${FONTS.title}; font-size: 18px; font-weight: 400; color: ${COLORS.accentSecondary}; line-height: 1.3;`}
              >
                {recipe.title}
              </h2>
              <button
                onClick={() => setSelected(null)}
                style={`background: none; border: none; font-family: ${FONTS.mono}; font-size: 20px; cursor: pointer; padding: 0; line-height: 1; color: ${COLORS.accent};`}
              >
                ✕
              </button>
            </div>

            {recipe.image ? (
              <img
                src={recipe.image}
                alt={recipe.title}
                style={`width: 100%; height: 200px; object-fit: cover; border-radius: 8px; margin-bottom: 20px; background: ${COLORS.surface}; border: 1px solid ${COLORS.border}; box-shadow: 4px 4px 4px rgba(0,0,0,0.2);`}
              />
            ) : (
              <div
                style={`width: 100%; height: 200px; border-radius: 8px; margin-bottom: 20px; background: ${COLORS.surface}; border: 1px solid ${COLORS.border}; box-shadow: 4px 4px 4px rgba(0,0,0,0.2); display: flex; align-items: center; justify-content: center;`}
              >
                <span
                  style={`font-family: ${FONTS.mono}; font-size: 14px; color: #555;`}
                >
                  Image placeholder
                </span>
              </div>
            )}

            {recipe.total_time > 0 && (
              <p
                style={`font-family: ${FONTS.mono}; font-size: 13px; color: ${COLORS.textMuted}; margin: 0 0 24px;`}
              >
                Total time: {recipe.total_time} min
              </p>
            )}

            <h3
              style={`font-family: ${FONTS.mono}; font-weight: 600; font-size: 16px; color: ${COLORS.textHeader}; margin: 0 0 8px;`}
            >
              Ingredients
            </h3>
            <ul
              style={`padding-left: 20px; margin: 0 0 24px; font-family: ${FONTS.body}; font-size: 14px; color: ${COLORS.textMuted}; line-height: 1.8;`}
            >
              {recipe.ingredients.map((ing) => (
                <li>{ing}</li>
              ))}
            </ul>

            <h3
              style={`font-family: ${FONTS.mono}; font-weight: 600; font-size: 16px; color: ${COLORS.textHeader}; margin: 0 0 8px;`}
            >
              Instructions
            </h3>
            <ol
              style={`padding-left: 20px; margin: 0 0 24px; font-family: ${FONTS.body}; font-size: 14px; color: ${COLORS.textMuted}; line-height: 1.8;`}
            >
              {recipe.instructions.map((step) => (
                <li style="margin-bottom: 4px;">{step}</li>
              ))}
            </ol>

            <a
              href={recipe.url}
              target="_blank"
              rel="noopener noreferrer"
              style={`display: inline-block; font-family: ${FONTS.title}; font-size: 13px; color: ${COLORS.accentSecondary}; text-decoration: none;`}
            >
              View original recipe ↗
            </a>
          </div>
        )}
      </Show>
    </div>
  );
};

export default App;
