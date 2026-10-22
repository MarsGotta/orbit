import type { Scenario } from "./scenario";

/** The list of scenarios, each a link to itself in both themes. */
export function ShowcaseIndex({ scenarios }: { scenarios: Scenario[] }) {
  return (
    <main style={{ padding: 32, fontFamily: "var(--o-font-sans, sans-serif)" }}>
      <h1>Orbit · escaparate</h1>
      <p>Cada escenario pinta la app con un estado inventado.</p>
      <ul>
        {scenarios.map((scenario) => (
          <li key={scenario.id}>
            <a href={`?s=${scenario.id}&tema=light`}>{scenario.id}</a> (
            <a href={`?s=${scenario.id}&tema=dark`}>oscuro</a>) · {scenario.view} · {scenario.state}
            : {scenario.description}
          </li>
        ))}
      </ul>
    </main>
  );
}
