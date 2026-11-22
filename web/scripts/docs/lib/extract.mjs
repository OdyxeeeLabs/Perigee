// extract.mjs — pulls a documentation model out of TypeScript/TSX source files
// using the TypeScript compiler API.
//
// Everything the docs site shows about a component's public surface is derived
// here, straight from the source of truth (the component file):
//
//   * exported components (function / `React.FC` / `React.forwardRef`)
//   * props: name, type text, optionality, description, default
//   * defaults taken from destructuring (`size = "md"`), overridable with
//     `@default` in JSDoc
//   * JSDoc description plus the `@example`, `@accessibility`, `@composition`,
//     `@since` and `@deprecated` tags
//   * exported union/object types referenced from props (variant/size enums)
//
// The module deliberately uses only `createSourceFile` (no full Program) so a
// docs build never has to type-check the whole app.

import { readFileSync } from 'node:fs';
import ts from 'typescript';

// ─────────────────────────────────────────────────────────────────────────────
// JSDoc helpers
// ─────────────────────────────────────────────────────────────────────────────

function commentToString(comment) {
  if (comment === undefined || comment === null) return '';
  if (typeof comment === 'string') return comment;
  if (Array.isArray(comment)) {
    return comment.map((part) => (typeof part === 'string' ? part : part.text ?? '')).join('');
  }
  return String(comment.text ?? '');
}

/** Finds the JSDoc block for a node, walking up to the wrapping statement. */
function findJsDoc(node) {
  let current = node;
  while (current) {
    const docs = current.jsDoc;
    if (docs && docs.length) return docs[docs.length - 1];
    if (ts.isSourceFile(current)) return null;
    current = current.parent;
  }
  return null;
}

/**
 * Normalises a JSDoc block into plain data.
 * `tags` keeps every tag verbatim (multi-valued tags become arrays) and
 * `params` keeps `@param` entries keyed by parameter name.
 */
export function parseJsDoc(jsDoc, sf) {
  const result = { description: '', tags: {}, params: {}, present: false };
  if (!jsDoc) return result;

  result.present = true;
  result.description = dedentBasic(commentToString(jsDoc.comment));

  for (const tag of jsDoc.tags ?? []) {
    const tagName = tag.tagName.text;
    if (tag.kind === ts.SyntaxKind.JSDocParameterTag) {
      const name = tag.name?.getText(sf) ?? '';
      result.params[name] = commentToString(tag.comment).trim();
      continue;
    }
    if (tag.kind === ts.SyntaxKind.JSDocDeprecatedTag) {
      push(result.tags, 'deprecated', commentToString(tag.comment).trim() || 'No reason provided.');
      continue;
    }
    push(result.tags, tagName, commentToString(tag.comment).trim());
  }

  return result;
}

function push(map, key, value) {
  if (!map[key]) map[key] = [];
  map[key].push(value);
}

function dedentBasic(text) {
  const lines = String(text ?? '').split('\n');
  while (lines.length && lines[0].trim() === '') lines.shift();
  while (lines.length && lines[lines.length - 1].trim() === '') lines.pop();
  const indents = lines.filter((l) => l.trim() !== '').map((l) => l.match(/^ */)[0].length);
  const shared = indents.length ? Math.min(...indents) : 0;
  return lines.map((l) => l.slice(shared)).join('\n');
}

/**
 * The first JSDoc block that appears before the first import/statement. Files
 * such as `ui/DataTable.tsx` document the component with a floating block above
 * the imports; that prose is still the component's headline documentation.
 */
function findFileDoc(sf) {
  const text = sf.getFullText();
  const firstImport = sf.statements.find((statement) => ts.isImportDeclaration(statement));
  // Only the region before the first import can hold file-level docs; anything
  // after it belongs to (and is found through) a declaration.
  const head = text.slice(0, firstImport ? firstImport.getStart(sf) : text.length);
  const match = /\/\*\*([\s\S]*?)\*\//.exec(head);
  if (!match) return null;
  return parseJsDocBlock(match[1]);
}

/** Parses a raw `/** … *​/` body (without the delimiters) as JSDoc. */
function parseJsDocBlock(raw) {
  // The comment needs a declaration to attach to: TypeScript only turns a
  // leading comment into a JSDoc node when it precedes a statement.
  const synthetic = ts.createSourceFile(
    'inline.ts',
    `/**${raw}*/\nconst __documented__ = 0;`,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TS,
  );
  const declaration = synthetic.statements[0]?.declarationList?.declarations?.[0];
  const jsDoc = declaration ? findJsDoc(declaration) : null;
  if (jsDoc) return parseJsDoc(jsDoc, synthetic);
  return { description: dedentBasic(raw.replace(/^\s*\*?\s?/gm, '')), tags: {}, params: {}, present: true };
}

// ─────────────────────────────────────────────────────────────────────────────
// Type helpers
// ─────────────────────────────────────────────────────────────────────────────

const ELEMENT_TYPE_MAP = {
  ButtonHTMLAttributes: 'button',
  InputHTMLAttributes: 'input',
  AnchorHTMLAttributes: 'a',
  TextareaHTMLAttributes: 'textarea',
  SelectHTMLAttributes: 'select',
  ImgHTMLAttributes: 'img',
  LabelHTMLAttributes: 'label',
  FormHTMLAttributes: 'form',
  TdHTMLAttributes: 'td',
  ThHTMLAttributes: 'th',
  LiHTMLAttributes: 'li',
  HTMLAttributes: null, // resolved from the generic argument (div, span, h3, …)
};

const DOM_INTERFACE_ELEMENT = /^(HTML|SVG)[A-Za-z]*(Element|Collection|Element\))?$/;

/** "React.ButtonHTMLAttributes<HTMLButtonElement>" -> "button". */
function elementFromTypeText(typeText) {
  const attr = /([A-Za-z]+HTMLAttributes|HTMLAttributes)\s*<\s*([A-Za-z0-9_$.]+)/.exec(typeText);
  if (attr) {
    const [, interfaceName, genericArg] = attr;
    if (Object.prototype.hasOwnProperty.call(ELEMENT_TYPE_MAP, interfaceName)) {
      if (ELEMENT_TYPE_MAP[interfaceName]) return ELEMENT_TYPE_MAP[interfaceName];
      const local = String(genericArg).split('.').pop();
      const derived = /^HTML([A-Za-z0-9]+)Element$/.exec(local);
      if (derived) return derived[1].toLowerCase();
      if (DOM_INTERFACE_ELEMENT.test(local)) return local.replace(/^HTML/, '').toLowerCase();
      return null;
    }
  }
  // React.forwardRef<HTMLDivElement, …> — the ref type names the element.
  const ref = /(?:^|[<,\s])(HTML|SVG)([A-Za-z0-9]+)Element/.exec(typeText);
  if (ref) return ref[2].toLowerCase();
  return null;
}

function unionValues(typeText) {
  const parts = splitTopLevel(typeText);
  if (parts.length < 2) return null;
  const values = [];
  for (const part of parts) {
    const trimmed = part.trim();
    const literal = /^(['"])(.*)\1$/.exec(trimmed);
    if (literal) values.push(literal[2]);
    else if (/^(true|false|-?\d+(\.\d+)?)$/.test(trimmed)) values.push(trimmed);
    else if (trimmed === 'undefined') values.push('undefined');
    else return null;
  }
  return values;
}

/** Splits `"a" | "b" | X<Y>` on top-level `|` only. */
function splitTopLevel(typeText) {
  const parts = [];
  let depth = 0;
  let current = '';
  for (const char of String(typeText)) {
    if (char === '<' || char === '(' || char === '{' || char === '[') depth += 1;
    if (char === '>' || char === ')' || char === '}' || char === ']') depth -= 1;
    if (char === '|' && depth === 0) {
      parts.push(current);
      current = '';
      continue;
    }
    current += char;
  }
  parts.push(current);
  return parts;
}

// ─────────────────────────────────────────────────────────────────────────────
// Declaration extraction
// ─────────────────────────────────────────────────────────────────────────────

function memberFromProperty(member, sf) {
  const doc = parseJsDoc(findJsDoc(member), sf);
  const name = member.name ? member.name.getText(sf) : '';
  const optional = Boolean(member.questionToken);
  let type = 'unknown';
  if (member.type) {
    type = collapseWhitespace(member.type.getText(sf));
  } else if (ts.isMethodSignature(member) && member.parameters) {
    const params = member.parameters.map((p) => p.getText(sf)).join(', ');
    type = `(${params}) => ${member.type ? member.type.getText(sf) : 'void'}`;
  }
  return {
    name,
    type,
    optional,
    description: doc.description,
    params: doc.params,
    defaultValue: doc.tags.default?.[0] ?? null,
    deprecated: doc.tags.deprecated?.[0] ?? null,
    since: doc.tags.since?.[0] ?? null,
    tags: doc.tags,
    kind: ts.isMethodSignature(member) ? 'method' : 'property',
  };
}

function collapseWhitespace(text) {
  return String(text ?? '')
    .replace(/\s*\n\s*/g, ' ')
    .replace(/\s{2,}/g, ' ')
    .trim();
}

function collectInterface(node, sf) {
  const doc = parseJsDoc(findJsDoc(node), sf);
  return {
    kind: 'interface',
    name: node.name.text,
    description: doc.description,
    tags: doc.tags,
    extends: (node.heritageClauses ?? [])
      .filter((clause) => clause.token === ts.SyntaxKind.ExtendsKeyword)
      .flatMap((clause) => clause.types.map((type) => collapseWhitespace(type.getText(sf)))),
    members: node.members
      .filter((member) => ts.isPropertySignature(member) || ts.isMethodSignature(member))
      .map((member) => memberFromProperty(member, sf)),
  };
}

function collectTypeAlias(node, sf) {
  const doc = parseJsDoc(findJsDoc(node), sf);
  const typeText = collapseWhitespace(node.type.getText(sf));
  const members = [];
  if (ts.isTypeLiteralNode(node.type)) {
    members.push(
      ...node.type.members
        .filter((member) => ts.isPropertySignature(member) || ts.isMethodSignature(member))
        .map((member) => memberFromProperty(member, sf)),
    );
  }
  return {
    kind: 'type',
    name: node.name.text,
    description: doc.description,
    tags: doc.tags,
    typeText,
    values: unionValues(typeText),
    members,
  };
}

/** Reads `{ a = 1, b: alias = 2, ...rest }` style destructuring defaults. */
function defaultsFromPattern(param, sf) {
  const defaults = new Map();
  let hasRest = false;
  if (!param || !param.name || !ts.isObjectBindingPattern(param.name)) {
    return { defaults, hasRest };
  }
  for (const element of param.name.elements) {
    if (ts.isSpreadAssignment(element) || element.dotDotDotToken) {
      hasRest = true;
      continue;
    }
    // `asChild: _asChild = false` binds the prop to a local alias — the doc row
    // must use the public prop name, not the alias.
    const name = element.propertyName?.getText(sf) ?? (element.name && element.name.getText(sf));
    if (!name) continue;
    if (element.initializer) {
      defaults.set(name, collapseWhitespace(element.initializer.getText(sf)));
    }
  }
  return { defaults, hasRest };
}

/** First JSX host element rendered by the component body (`<span>`, `<div>`…). */
function elementFromBody(body, sf) {
  if (!body) return null;
  let found = null;
  const visit = (node) => {
    if (found) return;
    if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) {
      const name = node.tagName.getText(sf);
      if (/^[a-z][a-zA-Z0-9]*$/.test(name)) {
        found = name;
        return;
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(body);
  return found;
}

function firstFunctionLike(node) {
  if (!node) return null;
  if (ts.isArrowFunction(node) || ts.isFunctionExpression(node)) return node;
  if (ts.isCallExpression(node)) {
    for (const arg of node.arguments) {
      const fn = firstFunctionLike(arg);
      if (fn) return fn;
    }
  }
  if (ts.isParenthesizedExpression(node)) return firstFunctionLike(node.expression);
  return null;
}

function propsTypeNodeFrom(declaration, initializer, sf) {
  // `const X: React.FC<Props> = …`
  if (declaration.type && ts.isTypeReferenceNode(declaration.type)) {
    const args = declaration.type.typeArguments;
    if (args && args.length) return args[args.length - 1];
  }
  // `React.forwardRef<Ref, Props>(…, …)`
  if (initializer && ts.isCallExpression(initializer) && initializer.typeArguments?.length) {
    return initializer.typeArguments[initializer.typeArguments.length - 1];
  }
  // `function Foo(props: Props)` / `const Foo = (props: Props) => …`
  const fn = ts.isFunctionDeclaration(declaration) ? declaration : firstFunctionLike(initializer);
  const param = fn?.parameters?.[0];
  if (param?.type) return param.type;
  return null;
}

function componentFrom(declaration, initializer, jsDocNode, sf, kind) {
  const doc = parseJsDoc(findJsDoc(jsDocNode ?? declaration), sf);
  const propsTypeNode = propsTypeNodeFrom(declaration, initializer, sf);
  const fn = ts.isFunctionDeclaration(declaration) ? declaration : firstFunctionLike(initializer);
  const { defaults, hasRest } = defaultsFromPattern(fn?.parameters?.[0], sf);
  const body = fn?.body;
  const start = (jsDocNode ?? declaration).getStart(sf);
  const { line } = sf.getLineAndCharacterOfPosition(start);

  return {
    name: declaration.name.text,
    kind,
    propsTypeName: propsTypeNode ? collapseWhitespace(propsTypeNode.getText(sf)) : null,
    propsTypeNode,
    defaults,
    hasRestProps: hasRest,
    bodyElement: elementFromBody(body, sf),
    doc,
    line: line + 1,
  };
}

// ─────────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Parses one component source file into a documentation model.
 *
 * @param {string} absPath absolute path of the file to parse
 * @param {string} relPath repo-relative path, used for source links
 */
export function extractFile(absPath, relPath) {
  const text = readFileSync(absPath, 'utf8');
  const sf = ts.createSourceFile(absPath, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);

  const file = {
    path: relPath,
    fileDoc: findFileDoc(sf),
    interfaces: new Map(),
    types: new Map(),
    components: new Map(),
    exportedTypes: new Set(),
  };

  for (const statement of sf.statements) {
    if (ts.isInterfaceDeclaration(statement)) {
      file.interfaces.set(statement.name.text, collectInterface(statement, sf));
      if (hasModifier(statement, ts.SyntaxKind.ExportKeyword)) file.exportedTypes.add(statement.name.text);
      continue;
    }
    if (ts.isTypeAliasDeclaration(statement)) {
      file.types.set(statement.name.text, collectTypeAlias(statement, sf));
      if (hasModifier(statement, ts.SyntaxKind.ExportKeyword)) file.exportedTypes.add(statement.name.text);
      continue;
    }
    if (ts.isFunctionDeclaration(statement) && statement.name) {
      file.components.set(
        statement.name.text,
        componentFrom(statement, null, statement, sf, 'function'),
      );
      continue;
    }
    if (ts.isVariableStatement(statement)) {
      for (const declaration of statement.declarationList.declarations) {
        if (!declaration.name || !ts.isIdentifier(declaration.name)) continue;
        if (!declaration.initializer) continue;
        const isForwardRef =
          ts.isCallExpression(declaration.initializer) &&
          /forwardRef$/.test(declaration.initializer.expression.getText(sf));
        const isTypedComponent = Boolean(declaration.type);
        const isFunctionish = Boolean(firstFunctionLike(declaration.initializer));
        if (!isForwardRef && !isTypedComponent && !isFunctionish) continue;
        file.components.set(
          declaration.name.text,
          componentFrom(declaration, declaration.initializer, statement, sf, isForwardRef ? 'forwardRef' : 'const'),
        );
      }
      continue;
    }
    if (ts.isExportDeclaration(statement) && !statement.moduleSpecifier && statement.exportClause) {
      for (const element of statement.exportClause.elements ?? []) {
        file.exportedTypes.add(element.name.text);
      }
    }
  }

  return file;
}

function hasModifier(node, kind) {
  return Boolean(node.modifiers?.some((modifier) => modifier.kind === kind));
}

// ─────────────────────────────────────────────────────────────────────────────
// Model resolution
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Resolves one exported component into the shape the renderers consume:
 * props rows (with defaults + descriptions), inherited attribute groups,
 * referenced exported types and accessibility/example metadata.
 */
export function resolveComponent(file, exportName) {
  const component = file.components.get(exportName);
  if (!component) return null;

  const propsTypeName = component.propsTypeName;
  // `DataTableProps<T>` / `React.HTMLAttributes<HTMLDivElement>` — look the
  // declaration up by its base name.
  const baseName = propsTypeName ? baseTypeName(propsTypeName) : null;
  const asInterface = baseName ? file.interfaces.get(baseName) : null;
  const asType = !asInterface && baseName ? file.types.get(baseName) : null;

  const declared = asInterface?.members ?? asType?.members ?? [];
  const inherited = asInterface?.extends ?? [];
  const inheritedFrom = inherited.length ? inherited : propsTypeName && !asInterface && !asType ? [propsTypeName] : [];

  const props = declared.map((member) => ({
    ...member,
    defaultValue: member.defaultValue ?? component.defaults.get(member.name) ?? null,
    hasDefault: member.defaultValue !== null || component.defaults.has(member.name),
    fromDestructuring: component.defaults.has(member.name),
  }));

  const restMember = component.hasRestProps
    ? {
        name: '…rest',
        type: inheritedFrom.length ? inheritedFrom.join(' & ') : 'native element attributes',
        optional: true,
        description: `Spread onto the underlying <${bodyElement(component, inheritedFrom) ?? 'element'}>. Any native attribute not listed above is forwarded verbatim.`,
        defaultValue: null,
        hasDefault: false,
        fromDestructuring: true,
        deprecated: null,
        since: null,
        tags: {},
        kind: 'rest',
      }
    : null;

  // Documentation precedence: the component's own JSDoc, then the JSDoc on its
  // props type (used by `LiveRegion.tsx`, which documents the interface), then
  // a file-level block (used by `ui/DataTable.tsx`).
  const ownDoc = component.doc;
  const propsDoc = asInterface
    ? { description: asInterface.description, tags: asInterface.tags }
    : asType
      ? { description: asType.description, tags: asType.tags }
      : { description: '', tags: {} };
  const fileDoc = file.fileDoc;
  const description = ownDoc.description || propsDoc.description || fileDoc?.description || '';
  const tags = {
    ...(fileDoc?.tags ?? {}),
    ...propsDoc.tags,
    ...ownDoc.tags,
  };
  for (const key of ['example', 'accessibility', 'composition', 'since', 'deprecated', 'note']) {
    const merged = [...(fileDoc?.tags?.[key] ?? []), ...(propsDoc.tags?.[key] ?? []), ...(ownDoc.tags?.[key] ?? [])];
    if (merged.length) tags[key] = merged;
    else delete tags[key];
  }

  const referencedTypes = collectReferencedTypes(file, props, propsTypeName);

  return {
    name: component.name,
    kind: component.kind,
    file: file.path,
    line: component.line,
    description,
    examples: [...(tags.example ?? [])],
    accessibility: [...(tags.accessibility ?? [])],
    composition: [...(tags.composition ?? [])],
    since: tags.since?.[0] ?? null,
    deprecated: tags.deprecated?.[0] ?? null,
    notes: tags.note ?? [],
    propsTypeName,
    props,
    restMember,
    inheritedFrom,
    element: bodyElement(component, inheritedFrom),
    types: referencedTypes,
    docTags: tags,
  };
}

function baseTypeName(typeText) {
  if (!typeText) return null;
  const withoutGenerics = String(typeText).replace(/<[\s\S]*>$/, '').trim();
  const parts = withoutGenerics.split('.');
  return parts[parts.length - 1] || null;
}

function bodyElement(component, inheritedFrom) {
  if (component.bodyElement) return component.bodyElement;
  for (const inherited of inheritedFrom) {
    const element = elementFromTypeText(inherited);
    if (element) return element;
  }
  return null;
}

function collectReferencedTypes(file, props, propsTypeName) {
  const names = new Set();
  const candidates = new Set([...file.exportedTypes, ...file.types.keys(), ...file.interfaces.keys()]);
  const baseName = baseTypeName(propsTypeName);
  for (const prop of props) {
    for (const name of candidates) {
      if (name === propsTypeName || name === baseName) continue;
      if (new RegExp(`\\b${escapeRegExp(name)}\\b`).test(prop.type)) names.add(name);
    }
  }
  const result = [];
  for (const name of names) {
    const type = file.types.get(name) ?? file.interfaces.get(name);
    if (type) result.push({ ...type, name });
  }
  return result.sort((a, b) => a.name.localeCompare(b.name));
}

function escapeRegExp(text) {
  return String(text).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
