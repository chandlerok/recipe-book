---
name: SolidJS
description: Use when building user interfaces with SolidJS, the signals-powered reactive UI framework. Covers the core library (solid-js), Solid Router, Solid Meta, and SolidStart meta-framework. Provides API references for signals, effects, stores, components, routing, data fetching, and rendering.
metadata:
  docs-url: https://docs.solidjs.com/
  llms-url: https://docs.solidjs.com/llms.txt
  version: "2.0"
---

# SolidJS Skill Reference

## Product Summary

SolidJS is a reactive JavaScript framework for building fast, efficient UIs with fine-grained reactivity. Unlike virtual-DOM frameworks, Solid compiles JSX templates into real DOM nodes and updates only what changes. Core packages: `solid-js` (runtime), `solid-router` (routing), `solid-meta` (head management), `solid-start` (meta-framework with file-based routing, server functions, SSR). Primary docs: https://docs.solidjs.com/

## When to Use

- **Building reactive UIs**: Components that update automatically when state changes, with no virtual DOM overhead
- **Fine-grained reactivity**: Signals, effects, memos, and stores for precise state management
- **Routing**: Client-side and server-side routing with Solid Router (lazy loading, nested layouts, path params, search params)
- **Data fetching**: Async data with `createResource`, server functions, Suspense boundaries
- **SSR/Streaming**: Server-side rendering, streaming, hydration for SEO and performance
- **Meta-framework**: SolidStart for file-based routing, API routes, middleware, sessions, and deployment

## Quick Reference

### Core Primitives

| Primitive | Purpose |
|-----------|---------|
| `createSignal(value)` | Basic reactive state, returns `[getter, setter]` |
| `createEffect(fn)` | Run side effects when reactive deps change |
| `createMemo(fn)` | Cached derived value, recalculates only when deps change |
| `createResource(source, fetcher)` | Async data fetching with Suspense integration |
| `createStore(value)` | Reactive store for nested/structured state |
| `createMutable(value)` | Mutable reactive store proxy |
| `createContext(default)` | Context for dependency injection |
| `createRoot(disposer)` | Non-tracked owner scope for manual memory management |

### Control Flow Components

| Component | Purpose |
|-----------|---------|
| `<Show when={} fallback={}>` | Conditional rendering |
| `<Switch>` / `<Match>` | Multi-condition branching |
| `<For each={} fallback={}>` | List rendering by identity |
| `<Index each={} fallback={}>` | List rendering by index |
| `<Dynamic component={}>` | Runtime component selection |
| `<ErrorBoundary fallback={}>` | Error boundary |
| `<Portal mount={}>` | Teleport to outside DOM hierarchy |
| `<Suspense fallback={}>` | Async dependency loading state |

### JSX Attributes

| Attribute | Purpose |
|-----------|---------|
| `classList={{ key: bool }}` | Toggle classes from an object |
| `style={obj}` or `style="str"` | Inline styles (object or string) |
| `ref={el}` | Capture DOM element ref |
| `on:event={fn}` | Direct `addEventListener` attachment |
| `onEvent={fn}` | Delegated event handler |
| `use:directive` | Attach a directive function |
| `prop:*` | Force JSX key as DOM property |
| `attr:*` | Force JSX key as HTML attribute |
| `innerHTML` / `textContent` | Set HTML or text content |
| `@once` | Mark expression as compile-time static |

### Lifecycle & Utilities

| API | Purpose |
|-----|---------|
| `onMount(fn)` | Run once after initial render |
| `onCleanup(fn)` | Register cleanup on scope disposal |
| `batch(fn)` | Group reactive updates |
| `untrack(fn)` | Read signals without tracking |
| `on(deps, fn, options)` | Explicit effect dependency control |
| `mergeProps(...sources)` | Merge prop objects reactively |
| `splitProps(props, ...keys)` | Split props into subsets |
| `lazy(() => import(...))` | Lazy-loaded component |
| `children(() => props.children)` | Resolve children prop as accessor |
| `createUniqueId()` | Unique ID for render/hydration context |

### Rendering (solid-js/web)

| API | Purpose |
|-----|---------|
| `render(Comp, mountEl)` | Mount root in browser |
| `hydrate(Comp, mountEl)` | Hydrate SSR HTML |
| `renderToString(Comp)` | Synchronous SSR |
| `renderToStringAsync(Comp)` | SSR with async suspense |
| `renderToStream(Comp)` | Stream SSR HTML |
| `isServer` / `isDev` / `DEV` | Environment flags |
| `HydrationScript` / `generateHydrationScript()` | SSR hydration bootstrap |
| `getRequestEvent()` | Read current request event |

### Store Utilities

| API | Purpose |
|-----|---------|
| `createStore(initial)` | Create reactive store + setter |
| `createMutable(initial)` | Create mutable store proxy |
| `produce(fn)` | Immutable-style store mutation |
| `reconcile(new, options)` | Diff and reconcile store data |
| `modifyMutable(store, fn)` | Batch apply to mutable store |
| `unwrap(store)` | Remove store proxy wrapping |

### Solid Router

| API | Purpose |
|-----|---------|
| `<Router>` | Browser-based routing context |
| `<Route path component>` | Route definition |
| `<A href>` | Navigation link with active state |
| `<Navigate href>` | Declarative navigation |
| `<HashRouter>` / `<MemoryRouter>` | Alternative router types |
| `useNavigate()` | Programmatic navigation |
| `useLocation()` | Current location |
| `useParams()` | Route parameters |
| `useSearchParams()` | Query string params + setter |
| `useMatch(pattern)` | Regex path matching |
| `useIsRouting()` | Detect route transition in progress |
| `useBeforeLeave(fn)` | Intercept navigation away |
| `createAsync(fn)` / `createAsyncStore(fn)` | Promise data accessors |
| `action(fn)` | Server mutation action |
| `query(fn)` / `cache(fn)` | Cached data query |
| `revalidate(...keys)` | Retrigger query cache entries |
| `useAction(action)` | Router-bound action caller |
| `useSubmission(action)` / `useSubmissions(action)` | Track form submissions |
| `preload` | Route-level data preloading |

### SolidStart (Meta-Framework)

| Concept | Description |
|---------|-------------|
| File-based routing | `src/routes/` directory structure |
| API routes | `src/routes/api/` for REST/GraphQL |
| `createHandler(fn)` | Server handler creation |
| `createMiddleware(fn)` | Middleware definitions |
| `"use server"` directive | Marks server functions |
| `defineConfig({...})` | App configuration (Vite + Nitro) |
| `<FileRoutes />` | Filesystem-generated route definitions |
| `<StartServer />` | SSR document component |
| `clientOnly(() => import(...))` | Client-only component wrapper |
| Middleware | Auth, logging, headers |
| Sessions | Encrypted cookie-based sessions |
| WebSocket | Real-time bidirectional communication |

## Common Patterns

### Reactive State
```jsx
const [count, setCount] = createSignal(0);
// Read: count()  — calls the getter
// Write: setCount(5) or setCount(prev => prev + 1)
```

### Derived Values
```jsx
const doubled = createMemo(() => count() * 2);
```

### Side Effects
```jsx
createEffect(() => console.log("Count:", count()));
```

### Conditional Rendering
```jsx
<Show when={loggedIn()} fallback={<Login />}>
  <Dashboard />
</Show>
```

### List Rendering
```jsx
<For each={items()}>{(item, index) =>
  <li>{index() + 1}. {item.name}</li>
}</For>
```

### Async Data (createResource)
```jsx
const [user] = createResource(() => userId(), fetchUser);
// <Suspense fallback={<Loading />}>
//   <Profile user={user()} />
// </Suspense>
```

### Store (Nested State)
```jsx
const [state, setState] = createStore({ todos: [] });
setState("todos", (t) => [...t, { id: 3, text: "New" }]);
```

### Context API
```jsx
const Counter = createContext();
const [count, setCount] = createSignal(0);
<Counter.Provider value={[count, setCount]}>
  <Children />
</Counter.Provider>
// Child: const [count, setCount] = useContext(Counter);
```

### Router Setup
```jsx
<Router root={App}>
  <Route path="/" component={Home} />
  <Route path="/users/:id" component={User} />
</Router>
```

### Form Actions (SolidStart)
```jsx
const submit = action(async (formData) => {
  "use server";
  // mutate DB
  return redirect("/success");
});
// <form action={submit} method="post">...</form>
```

### Routing from llms.txt

**Solid Router**
- [Installation and setup](https://docs.solidjs.com/solid-router/getting-started/installation-and-setup)
- [Component routing](https://docs.solidjs.com/solid-router/getting-started/component)
- [Config-based routing](https://docs.solidjs.com/solid-router/getting-started/config)
- [Linking routes](https://docs.solidjs.com/solid-router/getting-started/linking-routes)
- [Navigation](https://docs.solidjs.com/solid-router/concepts/navigation)
- [Path parameters](https://docs.solidjs.com/solid-router/concepts/path-parameters)
- [Search parameters](https://docs.solidjs.com/solid-router/concepts/search-parameters)
- [Nesting routes](https://docs.solidjs.com/solid-router/concepts/nesting)
- [Layouts](https://docs.solidjs.com/solid-router/concepts/layouts)
- [Actions](https://docs.solidjs.com/solid-router/concepts/actions)
- [SSR](https://docs.solidjs.com/solid-router/rendering-modes/ssr)
- [Single page apps](https://docs.solidjs.com/solid-router/rendering-modes/spa)
- [Data fetching queries](https://docs.solidjs.com/solid-router/data-fetching/queries)
- [Streaming](https://docs.solidjs.com/solid-router/data-fetching/streaming)
- [Revalidation](https://docs.solidjs.com/solid-router/data-fetching/revalidation)
- [Lazy loading](https://docs.solidjs.com/solid-router/advanced-concepts/lazy-loading)
- [Migration from v0.9.x](https://docs.solidjs.com/solid-router/guides/migration)
- [A component](https://docs.solidjs.com/solid-router/reference/components/a)
- [HashRouter](https://docs.solidjs.com/solid-router/reference/components/hash-router)
- [MemoryRouter](https://docs.solidjs.com/solid-router/reference/components/memory-router)
- [Navigate](https://docs.solidjs.com/solid-router/reference/components/navigate)
- [Route](https://docs.solidjs.com/solid-router/reference/components/route)
- [Router](https://docs.solidjs.com/solid-router/reference/components/router)
- [action](https://docs.solidjs.com/solid-router/reference/data-apis/action)
- [cache / query](https://docs.solidjs.com/solid-router/reference/data-apis/query)
- [createAsync](https://docs.solidjs.com/solid-router/reference/data-apis/create-async)
- [createAsyncStore](https://docs.solidjs.com/solid-router/reference/data-apis/create-async-store)
- [revalidate](https://docs.solidjs.com/solid-router/reference/data-apis/revalidate)
- [useAction](https://docs.solidjs.com/solid-router/reference/data-apis/use-action)
- [useSubmission](https://docs.solidjs.com/solid-router/reference/data-apis/use-submission)
- [useSubmissions](https://docs.solidjs.com/solid-router/reference/data-apis/use-submissions)
- [useBeforeLeave](https://docs.solidjs.com/solid-router/reference/primitives/use-before-leave)
- [useCurrentMatches](https://docs.solidjs.com/solid-router/reference/primitives/use-current-matches)
- [useIsRouting](https://docs.solidjs.com/solid-router/reference/primitives/use-is-routing)
- [useLocation](https://docs.solidjs.com/solid-router/reference/primitives/use-location)
- [useMatch](https://docs.solidjs.com/solid-router/reference/primitives/use-match)
- [useNavigate](https://docs.solidjs.com/solid-router/reference/primitives/use-navigate)
- [useParams](https://docs.solidjs.com/solid-router/reference/primitives/use-params)
- [useSearchParams](https://docs.solidjs.com/solid-router/reference/primitives/use-search-params)
- [json response helper](https://docs.solidjs.com/solid-router/reference/response-helpers/json)
- [redirect response helper](https://docs.solidjs.com/solid-router/reference/response-helpers/redirect)

**SolidStart**
- [Getting started](https://docs.solidjs.com/solid-start/getting-started)
- [File-based routing](https://docs.solidjs.com/solid-start/building-your-application/routing)
- [API routes](https://docs.solidjs.com/solid-start/building-your-application/api-routes)
- [Data fetching](https://docs.solidjs.com/solid-start/building-your-application/data-fetching)
- [Data mutation](https://docs.solidjs.com/solid-start/building-your-application/data-mutation)
- [Head and metadata](https://docs.solidjs.com/solid-start/building-your-application/head-and-metadata)
- [Route pre-rendering](https://docs.solidjs.com/solid-start/building-your-application/route-prerendering)
- [Middleware](https://docs.solidjs.com/solid-start/advanced/middleware)
- [Sessions](https://docs.solidjs.com/solid-start/advanced/session)
- [Auth](https://docs.solidjs.com/solid-start/advanced/auth)
- [WebSocket endpoints](https://docs.solidjs.com/solid-start/advanced/websocket)
- [Security](https://docs.solidjs.com/solid-start/guides/security)
- [Data fetching guide](https://docs.solidjs.com/solid-start/guides/data-fetching)
- [Data mutation guide](https://docs.solidjs.com/solid-start/guides/data-mutation)
- [Service workers](https://docs.solidjs.com/solid-start/guides/service-workers)
- [Migrating from v1](https://docs.solidjs.com/solid-start/migrating-from-v1)
- [clientOnly](https://docs.solidjs.com/solid-start/reference/client/client-only)
- [defineConfig](https://docs.solidjs.com/solid-start/reference/config/define-config)
- [app.config.ts](https://docs.solidjs.com/solid-start/reference/entrypoints/app-config)
- [app.tsx](https://docs.solidjs.com/solid-start/reference/entrypoints/app)
- [entry-client.tsx](https://docs.solidjs.com/solid-start/reference/entrypoints/entry-client)
- [entry-server.tsx](https://docs.solidjs.com/solid-start/reference/entrypoints/entry-server)
- [FileRoutes](https://docs.solidjs.com/solid-start/reference/routing/file-routes)
- [createHandler](https://docs.solidjs.com/solid-start/reference/server/create-handler)
- [createMiddleware](https://docs.solidjs.com/solid-start/reference/server/create-middleware)
- [GET](https://docs.solidjs.com/solid-start/reference/server/get)
- ["use server"](https://docs.solidjs.com/solid-start/reference/server/use-server)
- [HttpHeader](https://docs.solidjs.com/solid-start/reference/server/http-header)
- [HttpStatusCode](https://docs.solidjs.com/solid-start/reference/server/http-status-code)
- [StartServer](https://docs.solidjs.com/solid-start/reference/server/start-server)

### Reference from llms.txt

**Basic reactivity**
- [createSignal](https://docs.solidjs.com/reference/basic-reactivity/create-signal)
- [createEffect](https://docs.solidjs.com/reference/basic-reactivity/create-effect)
- [createMemo](https://docs.solidjs.com/reference/basic-reactivity/create-memo)
- [createResource](https://docs.solidjs.com/reference/basic-reactivity/create-resource)

**Component APIs**
- [children](https://docs.solidjs.com/reference/component-apis/children)
- [createContext](https://docs.solidjs.com/reference/component-apis/create-context)
- [createUniqueId](https://docs.solidjs.com/reference/component-apis/create-unique-id)
- [lazy](https://docs.solidjs.com/reference/component-apis/lazy)
- [useContext](https://docs.solidjs.com/reference/component-apis/use-context)

**Components**
- [Dynamic](https://docs.solidjs.com/reference/components/dynamic)
- [ErrorBoundary](https://docs.solidjs.com/reference/components/error-boundary)
- [For](https://docs.solidjs.com/reference/components/for)
- [Index](https://docs.solidjs.com/reference/components/index-component)
- [Portal](https://docs.solidjs.com/reference/components/portal)
- [Show](https://docs.solidjs.com/reference/components/show)
- [Suspense](https://docs.solidjs.com/reference/components/suspense)
- [SuspenseList](https://docs.solidjs.com/reference/components/suspense-list)
- [Switch / Match](https://docs.solidjs.com/reference/components/switch-and-match)

**JSX Attributes**
- [attr:*](https://docs.solidjs.com/reference/jsx-attributes/attr)
- [bool:*](https://docs.solidjs.com/reference/jsx-attributes/bool)
- [classList](https://docs.solidjs.com/reference/jsx-attributes/classlist)
- [innerHTML](https://docs.solidjs.com/reference/jsx-attributes/innerhtml)
- [on:*](https://docs.solidjs.com/reference/jsx-attributes/on)
- [prop:*](https://docs.solidjs.com/reference/jsx-attributes/prop)
- [ref](https://docs.solidjs.com/reference/jsx-attributes/ref)
- [style](https://docs.solidjs.com/reference/jsx-attributes/style)
- [textContent](https://docs.solidjs.com/reference/jsx-attributes/textcontent)
- [use:*](https://docs.solidjs.com/reference/jsx-attributes/use)

**Lifecycle**
- [onCleanup](https://docs.solidjs.com/reference/lifecycle/on-cleanup)
- [onMount](https://docs.solidjs.com/reference/lifecycle/on-mount)

**Reactive Utilities**
- [batch](https://docs.solidjs.com/reference/reactive-utilities/batch)
- [catchError](https://docs.solidjs.com/reference/reactive-utilities/catch-error)
- [createRoot](https://docs.solidjs.com/reference/reactive-utilities/create-root)
- [from](https://docs.solidjs.com/reference/reactive-utilities/from)
- [getOwner](https://docs.solidjs.com/reference/reactive-utilities/get-owner)
- [indexArray](https://docs.solidjs.com/reference/reactive-utilities/index-array)
- [mapArray](https://docs.solidjs.com/reference/reactive-utilities/map-array)
- [mergeProps](https://docs.solidjs.com/reference/reactive-utilities/merge-props)
- [observable](https://docs.solidjs.com/reference/reactive-utilities/observable)
- [on (utility)](https://docs.solidjs.com/reference/reactive-utilities/on-util)
- [runWithOwner](https://docs.solidjs.com/reference/reactive-utilities/run-with-owner)
- [splitProps](https://docs.solidjs.com/reference/reactive-utilities/split-props)
- [startTransition](https://docs.solidjs.com/reference/reactive-utilities/start-transition)
- [untrack](https://docs.solidjs.com/reference/reactive-utilities/untrack)
- [useTransition](https://docs.solidjs.com/reference/reactive-utilities/use-transition)

**Rendering**
- [render](https://docs.solidjs.com/reference/rendering/render)
- [hydrate](https://docs.solidjs.com/reference/rendering/hydrate)
- [renderToString](https://docs.solidjs.com/reference/rendering/render-to-string)
- [renderToStringAsync](https://docs.solidjs.com/reference/rendering/render-to-string-async)
- [renderToStream](https://docs.solidjs.com/reference/rendering/render-to-stream)
- [hydrationScript](https://docs.solidjs.com/reference/rendering/hydration-script)
- [isServer / isDev / DEV](https://docs.solidjs.com/reference/rendering/is-server)

**Secondary Primitives**
- [createComputed](https://docs.solidjs.com/reference/secondary-primitives/create-computed)
- [createDeferred](https://docs.solidjs.com/reference/secondary-primitives/create-deferred)
- [createReaction](https://docs.solidjs.com/reference/secondary-primitives/create-reaction)
- [createRenderEffect](https://docs.solidjs.com/reference/secondary-primitives/create-render-effect)
- [createSelector](https://docs.solidjs.com/reference/secondary-primitives/create-selector)

**Store Utilities**
- [createMutable](https://docs.solidjs.com/reference/store-utilities/create-mutable)
- [createStore](https://docs.solidjs.com/reference/store-utilities/create-store)
- [modifyMutable](https://docs.solidjs.com/reference/store-utilities/modify-mutable)
- [produce](https://docs.solidjs.com/reference/store-utilities/produce)
- [reconcile](https://docs.solidjs.com/reference/store-utilities/reconcile)
- [unwrap](https://docs.solidjs.com/reference/store-utilities/unwrap)

**Solid Meta**
- [MetaProvider](https://docs.solidjs.com/solid-meta/reference/meta/metaprovider)
- [Title](https://docs.solidjs.com/solid-meta/reference/meta/title)
- [Meta](https://docs.solidjs.com/solid-meta/reference/meta/meta)
- [Link](https://docs.solidjs.com/solid-meta/reference/meta/link)
- [Style](https://docs.solidjs.com/solid-meta/reference/meta/style)
- [Base](https://docs.solidjs.com/solid-meta/reference/meta/base)
- [useHead](https://docs.solidjs.com/solid-meta/reference/meta/use-head)

## Verification Checklist

- [ ] Signals created with `createSignal` use getter/setter pattern `[get, set]`
- [ ] Effects use `createEffect` for side effects, not inside render functions
- [ ] Derived values use `createMemo` to avoid redundant computation
- [ ] List keys use `<For>` for identity-based rendering, `<Index>` for index-based
- [ ] Async data uses `createResource` with `<Suspense>` boundary
- [ ] Store updates are immutable via setter function or `produce`
- [ ] Components are imported and used as JSX tags
- [ ] Router links use `<A>` component for proper SPA navigation
- [ ] Server functions are marked with `"use server"` directive
- [ ] Cleanup registered with `onCleanup` for subscriptions/timers
- [ ] `batch` wraps multiple signal updates to prevent intermediate re-renders
- [ ] `mergeProps` used for default/overridden prop patterns

## Resources

- **Full docs (LLMs.txt)**: https://docs.solidjs.com/llms.txt
- **Core docs**: https://docs.solidjs.com/
- **Solid Router**: https://docs.solidjs.com/solid-router
- **SolidStart**: https://docs.solidjs.com/solid-start
- **Solid Meta**: https://docs.solidjs.com/solid-meta
- **Reference API**: https://docs.solidjs.com/reference
- **Tutorial**: https://docs.solidjs.com/quick-start
- **Guides** (testing, deployment, state management, data fetching): https://docs.solidjs.com/guides
- **GitHub**: https://github.com/solidjs/solid
- **Discord**: https://discord.com/invite/solidjs
