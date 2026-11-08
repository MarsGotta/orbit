import { english } from "./en";
import { spanish } from "./es";
import { defineCatalogue, type Catalogue } from "./catalogue";

const complete: Catalogue = english("Atlas");
const spanishComplete: Catalogue = spanish("Atlas");
const { shell: _shell, ...missing } = complete;
// @ts-expect-error A catalogue missing a screen cannot be supplied to the provider.
const missingCatalogue: Catalogue = missing;
// @ts-expect-error Extra catalogue keys are rejected.
const extraCatalogue: Catalogue = { ...complete, extraScreen: "Extra" };
void [spanishComplete, _shell, missingCatalogue, extraCatalogue];

const { record: _record, ...shellMissingKey } = complete.shell;
// @ts-expect-error A missing leaf is rejected by the same public catalogue contract.
defineCatalogue({ ...complete, shell: shellMissingKey });
// @ts-expect-error Extra keys in composed topic objects are rejected, including spreads.
defineCatalogue({ ...complete, shell: { ...complete.shell, extraLabel: "Extra" } });
void _record;

const { es: _spanishLanguage, ...languagesMissingKey } = complete.notes.languages;
// @ts-expect-error Static map keys remain required even when callers can look up future keys.
defineCatalogue({ ...complete, notes: { ...complete.notes, languages: languagesMissingKey } });
void _spanishLanguage;
