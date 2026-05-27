import { createSignal, createResource } from "solid-js";

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

async function searchRecipes(query: string): Promise<SearchHit[]> {
  if (!query.trim()) return [];
  const res = await fetch(
    `/api/recipes/search?q=${encodeURIComponent(query)}&limit=20`,
  );
  if (!res.ok) return [];
  return res.json();
}

const App = () => {
  const [query, setQuery] = createSignal("");
  const [hits] = createResource(query, searchRecipes, { deferStream: true });

  return (
    <div style="max-width: 640px; margin: 0 auto; padding: 2rem 1rem; font-family: system-ui, sans-serif;">
      <h1 style="font-size: 1.5rem; margin-bottom: 1rem;">Recipe Search</h1>

      <input
        type="text"
        placeholder="Search recipes..."
        value={query()}
        onInput={(e) => setQuery(e.currentTarget.value)}
        style="width: 100%; padding: 0.75rem; font-size: 1rem; border: 1px solid #ccc; border-radius: 6px; box-sizing: border-box;"
      />

      <ul style="list-style: none; padding: 0; margin-top: 1rem;">
        {hits()?.map((hit) => (
          <li style="padding: 0.75rem; border-bottom: 1px solid #eee; display: flex; gap: 1rem;">
            {hit.recipe.image && (
              <img
                src={hit.recipe.image}
                alt={hit.recipe.title}
                style="width: 80px; height: 80px; object-fit: cover; border-radius: 4px; flex-shrink: 0;"
              />
            )}
            <div>
              <a
                href={hit.recipe.url}
                target="_blank"
                rel="noopener noreferrer"
                style="font-weight: 600; color: #1a73e8; text-decoration: none;"
              >
                {hit.recipe.title}
              </a>
              {hit.recipe.total_time > 0 && (
                <p style="margin: 0.25rem 0 0; font-size: 0.85rem; color: #666;">
                  {hit.recipe.total_time} min
                </p>
              )}
            </div>
          </li>
        ))}
      </ul>

      {query() && hits() && hits()!.length === 0 && (
        <p style="color: #888; text-align: center; margin-top: 2rem;">
          No recipes found
        </p>
      )}
    </div>
  );
};

export default App;
