import { readFile } from 'node:fs/promises';
import { join } from 'node:path';

import init, { HtmlConverter } from '../pkg/ironpress.js';

const wasm = await readFile(new URL('../pkg/ironpress_bg.wasm', import.meta.url));
await init({ module_or_path: wasm });

const cargoManifest = await readFile(new URL('../Cargo.toml', import.meta.url), 'utf8');
const packageManifest = JSON.parse(
  await readFile(new URL('../pkg/package.json', import.meta.url), 'utf8'),
);
const crateVersion = cargoManifest.match(
  /^version\s*=\s*"([^"]+)"/m,
)?.[1];
if (packageManifest.version !== crateVersion) {
  throw new Error(
    `npm version ${packageManifest.version} does not match crate version ${crateVersion}`,
  );
}

const portableMethods = [
  'pageSize',
  'pageSizeCustom',
  'margin',
  'marginSides',
  'compress',
  'jpegQuality',
  'autoResizeImages',
  'imageDpi',
  'filterDpi',
  'maskDpi',
  'backgroundRasterDpi',
  'occlusionCull',
  'sanitize',
  'accessible',
  'documentTitle',
  'documentLanguage',
  'addFont',
  'addFontPack',
  'header',
  'footer',
  'htmlToPdf',
  'markdownToPdf',
];

const configured = new HtmlConverter();
for (const method of portableMethods) {
  if (typeof configured[method] !== 'function') {
    throw new Error(`HtmlConverter.${method} is missing from the portable binding contract`);
  }
}
configured.pageSizeCustom(320, 480);
configured.marginSides(12, 13, 14, 15);
configured.compress(false);
configured.jpegQuality(82);
configured.autoResizeImages(false);
configured.imageDpi(144);
configured.filterDpi(96);
configured.maskDpi(144);
configured.backgroundRasterDpi(120);
configured.occlusionCull(true);
configured.sanitize(true);
configured.header('Contract header');
configured.footer('Page {page} of {pages}');

const configuredPdf = Buffer.from(configured.htmlToPdf('<h1>WASM binding</h1>'));
if (!configuredPdf.subarray(0, 4).equals(Buffer.from('%PDF'))) {
  throw new Error('configured WASM conversion did not produce a PDF');
}
if (!configuredPdf.includes(Buffer.from('/MediaBox [0 0 320 480]'))) {
  throw new Error('pageSizeCustom did not reach the WASM conversion');
}
configured.accessible(true);
configured.documentTitle('Contract');
configured.documentLanguage('en-GB');
const accessiblePdf = Buffer.from(configured.htmlToPdf('<p>Accessible</p>'));
if (!accessiblePdf.includes(Buffer.from('/StructTreeRoot'))) {
  throw new Error('accessible output did not reach the WASM conversion');
}
let invalidLanguageWasRejected = false;
try {
  configured.documentLanguage('1234');
} catch {
  invalidLanguageWasRejected = true;
}
if (!invalidLanguageWasRejected) {
  throw new Error('an invalid language tag was accepted');
}
configured.free();

const artifactDirectory = process.argv[2];
const cases = artifactDirectory
  ? [
      ['cjk-jp', 'ironpress-font-cjk-jp.ttf', "<p lang='ja'>日本語</p>", 'NotoSansJP'],
      ['cjk-kr', 'ironpress-font-cjk-kr.ttf', "<p lang='ko'>안녕하세요</p>", 'NotoSansKR'],
      ['cjk-sc', 'ironpress-font-cjk-sc.ttf', "<p lang='zh-Hans'>简体中文</p>", 'NotoSansSC'],
      ['cjk-tc', 'ironpress-font-cjk-tc.ttf', "<p lang='zh-Hant'>繁體中文</p>", 'NotoSansTC'],
      ['emoji', 'ironpress-font-emoji.ttf', '<p>😀</p>', 'NotoEmoji'],
    ]
  : [
      [
        'cjk-jp',
        new URL('../tests/fonts/IronpressCjkVertical.ttf', import.meta.url),
        "<p lang='ja'>第</p>",
        'DroidSansFallback',
      ],
    ];

for (const [kind, source, html, expectedFont] of cases) {
  const converter = new HtmlConverter();
  const bytes = await readFile(artifactDirectory ? join(artifactDirectory, source) : source);
  converter.addFontPack(kind, bytes);

  const rawPdf = Buffer.from(converter.htmlToPdf(html)).toString('latin1');
  if (!rawPdf.includes(expectedFont)) {
    throw new Error(`${kind} did not embed ${expectedFont}`);
  }
  converter.free();
}
