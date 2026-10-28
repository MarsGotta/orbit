// Work inventory, not a gate: Possible inline copy literals need a human
// pass before translation. AST parsing excludes comments and type syntax.
import ts from "typescript";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../src/features/", import.meta.url));
const counts = new Map();
const spanish =
  /[áéíóúüñ¿¡]|\b(?:el|la|los|las|un|una|tus|tu|para|con|sin|que|no|del|al|de|se|por|en|hay|como|hoy|ayer|guardar|cancelar|cerrar|abrir|buscar|borrar|cambiar|asistente|ajustes|reuniones|notas|copiar|reintentar|continuar|volver|nombre|ayuda|listo|cargando|enviar|ver|editar|quitar|crear|tarea|proyecto|personas|archivos|conectar)\b/i;

function isType(node) {
  let parent = node.parent;
  while (parent) {
    if (ts.isTypeNode(parent) || ts.isImportDeclaration(parent)) return true;
    parent = parent.parent;
  }
  return false;
}
function literalText(node) {
  if (ts.isJsxText(node)) return node.text.trim();
  if (ts.isStringLiteralLike(node)) return node.text;
  if (ts.isTemplateExpression(node))
    return node.head.text + node.templateSpans.map((s) => s.literal.text).join(" ");
  return null;
}
function countFile(file) {
  const source = ts.createSourceFile(
    file,
    fs.readFileSync(file, "utf8"),
    ts.ScriptTarget.Latest,
    true,
  );
  let count = 0;
  function visit(node) {
    const text = literalText(node);
    if (text !== null && !isType(node)) {
      const parent = node.parent;
      const structural =
        (ts.isPropertyAssignment(parent) && parent.name === node) ||
        ts.isCaseClause(parent) ||
        (ts.isBinaryExpression(parent) &&
          [
            ts.SyntaxKind.EqualsEqualsToken,
            ts.SyntaxKind.EqualsEqualsEqualsToken,
            ts.SyntaxKind.ExclamationEqualsToken,
            ts.SyntaxKind.ExclamationEqualsEqualsToken,
            ts.SyntaxKind.InKeyword,
          ].includes(parent.operatorToken.kind)) ||
        (ts.isJsxAttribute(parent) &&
          !/^(title|label|help|placeholder|alt|aria-label)$/.test(parent.name.text));
      const prose = /[\p{L}].*\s.*[\p{L}]|[áéíóúüñ¿¡]/u.test(text);
      const uiText =
        ts.isJsxText(node) ||
        ts.isJsxAttribute(parent) ||
        (ts.isPropertyAssignment(parent) &&
          /^(title|label|help|placeholder|message|description|text|heading|body|lead|hint|detail)$/.test(
            parent.name.getText(source),
          ));
      const human = /[\p{L}]/u.test(text) && (uiText || prose || spanish.test(text));
      if (!structural && human && !/^https?:|^var\(--|^#\//.test(text)) count++;
      if (ts.isTemplateExpression(node)) {
        for (const span of node.templateSpans) visit(span.expression);
        return;
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
  const feature = path.relative(root, file).split(path.sep)[0];
  counts.set(feature, (counts.get(feature) ?? 0) + count);
}
function walk(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory() && !entry.name.startsWith("__")) walk(file);
    else if (/\.tsx?$/.test(entry.name) && !/(\.test\.|^test[-.])/.test(entry.name))
      countFile(file);
  }
}
walk(root);
console.log("| Feature folder | Inline copy candidates |\n|---|---:|");
for (const [feature, count] of [...counts].sort()) console.log(`| ${feature} | ${count} |`);
console.log(`| **Total** | **${[...counts.values()].reduce((a, b) => a + b, 0)}** |`);
