import { createSignal, createResource, Show } from "solid-js";

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

  const data = () => results();
  const hits = () => data()?.hits ?? [];
  const total = () => data()?.total ?? 0;
  const pageCount = () => Math.ceil(total() / 20) || 1;
  const currentPage = () => Math.floor(offset() / 20) + 1;

  return (
    <div style="max-width: 640px; margin: 0 auto; padding: 2rem 1rem; font-family: system-ui, sans-serif;">
      <h1 style="font-size: 1.5rem; margin-bottom: 1rem;">Recipe Search</h1>

      <input
        type="text"
        placeholder="Search recipes..."
        value={query()}
        onInput={onInput}
        style="width: 100%; padding: 0.75rem; font-size: 1rem; border: 1px solid #ccc; border-radius: 6px; box-sizing: border-box;"
      />

      <Show when={hits().length > 0}>
        <ul style="list-style: none; padding: 0; margin-top: 1rem;">
          {hits().map((hit) => (
            <li
              onClick={() => setSelected(hit.recipe)}
              style="padding: 0.75rem; border-bottom: 1px solid #eee; display: flex; gap: 1rem; cursor: pointer;"
            >
              {hit.recipe.image && (
                <img
                  src={hit.recipe.image}
                  alt={hit.recipe.title}
                  style="width: 80px; height: 80px; object-fit: cover; border-radius: 4px; flex-shrink: 0;"
                />
              )}
              <div>
                <span style="font-weight: 600; color: #1a73e8;">
                  {hit.recipe.title}
                </span>
                {hit.recipe.total_time > 0 && (
                  <p style="margin: 0.25rem 0 0; font-size: 0.85rem; color: #666;">
                    {hit.recipe.total_time} min
                  </p>
                )}
              </div>
            </li>
          ))}
        </ul>

        <div style="display: flex; align-items: center; justify-content: center; gap: 1rem; margin-top: 1rem;">
          <button
            onClick={() => goToPage(currentPage() - 1)}
            disabled={currentPage() <= 1}
            style="padding: 0.4rem 0.8rem; border: 1px solid #ccc; border-radius: 4px; background: #fff; cursor: pointer; font-size: 0.9rem;"
          >
            ◀ Prev
          </button>
          <span style="font-size: 0.9rem; color: #555;">
            Page {currentPage()} of {pageCount()} ({total()} results)
          </span>
          <button
            onClick={() => goToPage(currentPage() + 1)}
            disabled={currentPage() >= pageCount()}
            style="padding: 0.4rem 0.8rem; border: 1px solid #ccc; border-radius: 4px; background: #fff; cursor: pointer; font-size: 0.9rem;"
          >
            Next ▶
          </button>
        </div>
      </Show>

      <Show when={query().trim() && hits().length === 0 && !results.loading}>
        <p style="color: #888; text-align: center; margin-top: 2rem;">
          No recipes found
        </p>
      </Show>

      <Show when={selected()} keyed>
        {(recipe) => (
          <>
            <div
              onClick={() => setSelected(null)}
              style="position: fixed; inset: 0; background: rgba(0,0,0,0.3); z-index: 998;"
            />
            <div style="position: fixed; top: 0; right: 0; bottom: 0; width: min(480px, 100vw); background: #fff; z-index: 999; overflow-y: auto; box-shadow: -4px 0 12px rgba(0,0,0,0.15); padding: 1.5rem; box-sizing: border-box; font-family: system-ui, sans-serif;">
              <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1rem;">
                <h2 style="margin: 0; font-size: 1.25rem; line-height: 1.3;">
                  {recipe.title}
                </h2>
                <button
                  onClick={() => setSelected(null)}
                  style="background: none; border: none; font-size: 1.5rem; cursor: pointer; padding: 0; line-height: 1; color: #888;"
                >
                  ✕
                </button>
              </div>

              {recipe.image && (
                <img
                  src={recipe.image}
                  alt={recipe.title}
                  style="width: 100%; max-height: 300px; object-fit: cover; border-radius: 8px; margin-bottom: 1rem;"
                />
              )}

              {recipe.total_time > 0 && (
                <p style="font-size: 0.9rem; color: #666; margin-bottom: 1rem;">
                  Total time: {recipe.total_time} min
                </p>
              )}

              <h3 style="font-size: 1rem; margin-bottom: 0.5rem;">
                Ingredients
              </h3>
              <ul style="padding-left: 1.25rem; margin-bottom: 1.25rem; line-height: 1.6;">
                {recipe.ingredients.map((ing) => (
                  <li>{ing}</li>
                ))}
              </ul>

              <h3 style="font-size: 1rem; margin-bottom: 0.5rem;">
                Instructions
              </h3>
              <ol style="padding-left: 1.25rem; margin-bottom: 1.25rem; line-height: 1.6;">
                {recipe.instructions.map((step) => (
                  <li style="margin-bottom: 0.5rem;">{step}</li>
                ))}
              </ol>

              <a
                href={recipe.url}
                target="_blank"
                rel="noopener noreferrer"
                style="display: inline-block; margin-top: 0.5rem; color: #1a73e8; font-size: 0.9rem;"
              >
                View original recipe ↗
              </a>
            </div>
          </>
        )}
      </Show>
    </div>
  );
};

export default App;
