import ts from "typescript";
import { exceptions } from "./inline-copy-exceptions.mjs";

const attributes =
  /^(title|label|help|lead|placeholder|alt|aria-label|text|hint|description|heading|body|pendingLabel|emptyLabel)$/;
const spanish =
  /[áéíóúüñ¿¡]|\b(?:el|la|los|las|un|una|tus|tu|para|con|sin|que|no|del|al|de|se|por|en|hay|como|hoy|ayer|guardar|cancelar|cerrar|abrir|buscar|borrar|cambiar|asistente|ajustes|reuniones|notas|copiar|reintentar|continuar|volver|nombre|ayuda|listo|cargando|enviar|ver|editar|quitar|crear|tarea|proyecto|personas|archivos|conectar)\b/i;
const comparisons = [
  ts.SyntaxKind.EqualsEqualsToken,
  ts.SyntaxKind.EqualsEqualsEqualsToken,
  ts.SyntaxKind.ExclamationEqualsToken,
  ts.SyntaxKind.ExclamationEqualsEqualsToken,
  ts.SyntaxKind.InKeyword,
];

function structural(node) {
  if (ts.isJsxText(node)) return false;
  const parent = node.parent;
  if ((ts.isPropertyAssignment(parent) && parent.name === node) || ts.isCaseClause(parent))
    return true;
  if (ts.isBinaryExpression(parent) && comparisons.includes(parent.operatorToken.kind)) return true;
  for (let at = parent; at; at = at.parent) {
    // A returned element renders its own children, even inside a components callback.
    if (ts.isJsxElement(at) || ts.isJsxFragment(at)) return false;
    if (
      ts.isVariableDeclaration(at) &&
      /^(FIELDS|FOCUSABLE|SYSTEM_LIGHT|REDUCE)$/.test(at.name.getText())
    )
      return true;
    if (ts.isTypeNode(at) || ts.isImportDeclaration(at)) return true;
    if (ts.isJsxAttribute(at)) return !attributes.test(at.name.getText());
    if (ts.isNewExpression(at) && at.expression.getText() === "Error") return true;
    if (
      ts.isCallExpression(at) &&
      /\.(querySelector(?:All)?|closest|matches|matchMedia)$/.test(at.expression.getText())
    )
      return true;
    if (ts.isCallExpression(at) && at.expression.getText() === "navigator.userAgent.includes")
      return true;
  }
  return false;
}
function technical(file, text, uiText) {
  if (exceptions[file]?.includes(text)) return true;
  if (uiText) return false;
  // CSS declarations, parser node names and keyboard bindings are not labels.
  if (file === "features/editor/theme.ts") return true;
  if (
    file === "features/editor/markdown-syntax.ts" &&
    /^(Highlight|Emphasis|Link|Mention)$/.test(text)
  )
    return true;
  if (file === "features/editor/editor-keys.ts" && /^(Enter|Tab|Escape|Backspace)$/.test(text))
    return true;
  if (file === "features/editor/live-preview.ts" && text === "cm-md-task cm-md-done") return true;
  if (file === "design/Brand.tsx" && /^M[\d.]+[ ,]/.test(text)) return true;
  return /^https?:|^var\(--|^#\/|^[a-z]{2}-[A-Z]{2}$/.test(text);
}
function renderedExpression(node) {
  for (let at = node.parent; at; at = at.parent) {
    if (ts.isJsxExpression(at)) return true;
    if (ts.isCallExpression(at) || ts.isArrowFunction(at) || ts.isBlock(at)) return false;
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

/** Inventory actual syntax, including helper copy; comments and types never count. */
export function inlineCopy(file, code) {
  const source = ts.createSourceFile(file, code, ts.ScriptTarget.Latest, true);
  const found = [];
  function visit(node) {
    const text = literalText(node);
    if (text !== null && !structural(node)) {
      const parent = node.parent;
      const prose = /[\p{L}].*\s.*[\p{L}]|[áéíóúüñ¿¡]/u.test(text);
      const uiText =
        ts.isJsxText(node) ||
        ts.isJsxAttribute(parent) ||
        renderedExpression(node) ||
        (ts.isPropertyAssignment(parent) && attributes.test(parent.name.getText(source)));
      const human =
        /[\p{L}]/u.test(text) &&
        (uiText || prose || spanish.test(text) || /^[A-Z][a-z]+(?:[ :,.…]|$)/.test(text));
      if (human && !technical(file, text, uiText))
        found.push({
          text,
          line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
        });
      if (ts.isTemplateExpression(node)) {
        for (const span of node.templateSpans) visit(span.expression);
        return;
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
  return found;
}
