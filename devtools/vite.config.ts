import path from "node:path";
import preactPlugin from "@preact/preset-vite";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite-plus";

const rootDir = import.meta.dirname;

export default defineConfig({
  plugins: [preactPlugin(), tailwindcss()],
  build: {
    minify: "oxc",
    target: "esnext",
    rolldownOptions: {
      input: {
        index: path.resolve(rootDir, "./index.html"),
      },
      output: {
        manualChunks(moduleId) {
          const chunks: Record<string, string[]> = {
            preact: ["preact"],
            "preact-iso": ["preact-iso"],
          };
          const matched = Object.entries(chunks).find(([, modules]) =>
            modules.some((mod) => moduleId.includes(`node_modules/${mod}`)),
          );
          return matched?.[0];
        },
      },
    },
  },
  resolve: {
    alias: {
      "@": path.resolve(rootDir, "./src"),
    },
  },
  test: {
    include: ["src/**/*.test.ts", "src/**/*.prop.ts"],
    globals: true,
    environment: "jsdom",
  },
  lint: {
    options: {
      typeAware: true,
      typeCheck: true,
    },
    plugins: ["typescript", "oxc", "unicorn", "import", "promise", "react", "vitest"],
    categories: {
      // 明らかに間違っているコード
      correctness: "error",
      // パフォーマンスに影響するコード
      perf: "error",
      // 疑わしいコード
      suspicious: "error",
      // 厳格なルール
      pedantic: "error",
      // 制限ルールは個別に設定
      restriction: "off",
      // スタイルルール
      style: "error",
    },
    rules: {
      // ===== eslint: 危険なコードの禁止 =====
      // DevTools アプリケーションとして console.log によるデバッグ情報の出力を許可する
      // restriction カテゴリでデフォルト off のため個別設定は不要
      // debugger 文の使用を禁止
      "no-debugger": "error",
      // alert/confirm/prompt の使用を禁止
      "no-alert": "error",
      // eval() の使用を禁止
      "no-eval": "error",
      // new Function() の使用を禁止
      "no-new-func": "error",
      // javascript: URL の使用を禁止
      "no-script-url": "error",
      // with 文の使用を禁止
      "no-with": "error",

      // ===== eslint: 比較と変数 =====
      // 厳密等価演算子 (===, !==) を強制
      eqeqeq: "error",
      // var の使用を禁止 (let/const を使用)
      "no-var": "error",
      // if/else/for/while で波括弧を強制
      curly: "error",
      // for-in ループで hasOwnProperty チェックを強制
      "guard-for-in": "error",

      // ===== eslint: 非推奨機能の禁止 =====
      // arguments.caller/callee の使用を禁止
      "no-caller": "error",
      // ネイティブオブジェクトの拡張を禁止
      "no-extend-native": "error",
      // 不要な bind() の使用を禁止
      "no-extra-bind": "error",
      // __iterator__ プロパティの使用を禁止
      "no-iterator": "error",
      // ラベル付き文の使用を禁止
      "no-labels": "error",
      // 不要なブロックの使用を禁止
      "no-lone-blocks": "error",
      // 複数行文字列 (バックスラッシュ) の使用を禁止
      "no-multi-str": "error",
      // プリミティブラッパーの new を禁止
      "no-new-wrappers": "error",
      // __proto__ プロパティの使用を禁止
      "no-proto": "error",

      // ===== eslint: コード品質 =====
      // return 文での代入を禁止
      "no-return-assign": "error",
      // 自己比較を禁止
      "no-self-compare": "error",
      // カンマ演算子の使用を禁止
      "no-sequences": "error",
      // リテラル値の throw を禁止 (Error オブジェクトを使用)
      "no-throw-literal": "error",
      // 未使用の式を禁止
      "no-unused-expressions": "error",
      // 不要な call()/apply() を禁止
      "no-useless-call": "error",
      // 不要な文字列連結を禁止
      "no-useless-concat": "error",

      // ===== eslint: モダン構文の推奨 =====
      // Math.pow() より ** 演算子を推奨
      "prefer-exponentiation-operator": "error",
      // Object.assign() よりスプレッド構文を推奨
      "prefer-object-spread": "error",
      // arguments より rest パラメータを推奨
      "prefer-rest-params": "error",
      // apply() よりスプレッド構文を推奨
      "prefer-spread": "error",
      // 文字列連結よりテンプレートリテラルを推奨
      "prefer-template": "error",
      // parseInt() で基数を明示
      radix: "error",
      // Symbol に説明を必須
      "symbol-description": "error",

      // ===== eslint: 複雑度の制限 =====
      // 循環的複雑度を制限 (分岐が多すぎる関数を検出)
      complexity: ["error", { max: 20 }],
      // 関数内のステートメント数を制限
      "max-statements": ["error", { max: 50 }],

      // ===== eslint: パフォーマンスと正確性 =====
      // 配列メソッドのコールバックで return を強制
      "array-callback-return": "error",
      // コンストラクタでの return を禁止
      "no-constructor-return": "error",
      // Promise executor での return を禁止
      "no-promise-executor-return": "error",
      // Object.prototype メソッドの直接呼び出しを禁止
      "no-prototype-builtins": "error",
      // var のブロックスコープ外使用を禁止
      "block-scoped-var": "error",
      // new の結果を使用しない場合を禁止
      "no-new": "error",
      // 不要なコンストラクタを禁止
      "no-useless-constructor": "error",

      // ===== eslint: スタイル =====
      // switch の default を最後に配置
      "default-case-last": "error",
      // デフォルト引数を最後に配置
      "default-param-last": "error",
      // getter/setter をグループ化
      "grouped-accessor-pairs": "error",
      // 不要な計算プロパティを禁止
      "no-useless-computed-key": "error",
      // Object.hasOwn() を推奨
      "prefer-object-has-own": "error",
      // parseInt より数値リテラルを推奨
      "prefer-numeric-literals": "error",
      // アロー関数の本体スタイルを統一
      "arrow-body-style": "error",
      // Yoda 条件を禁止 (if (5 === x) → if (x === 5))
      yoda: "error",
      // 不要な else を禁止
      "no-else-return": "error",
      // new Object() を禁止
      "no-object-constructor": "error",
      // 不要な return を禁止
      "no-useless-return": "error",
      // 引数の再代入を禁止
      "no-param-reassign": "error",
      // this を使用しないメソッドを検出
      "class-methods-use-this": "error",

      // ===== typescript: 非同期処理 =====
      // 非 Promise の await を禁止
      "typescript/await-thenable": "error",
      // 配列の delete を禁止 (splice を使用)
      "typescript/no-array-delete": "error",
      // toString() が意味のある値を返さないオブジェクトを検出
      "typescript/no-base-to-string": "error",
      // 紛らわしい void 式を禁止
      "typescript/no-confusing-void-expression": "error",
      // 非推奨 API の使用を警告
      "typescript/no-deprecated": "error",
      // 重複する型構成要素を禁止
      "typescript/no-duplicate-type-constituents": "error",
      // 未処理の Promise を禁止
      "typescript/no-floating-promises": "error",
      // 配列への for-in を禁止
      "typescript/no-for-in-array": "error",
      // 暗黙の eval を禁止
      "typescript/no-implied-eval": "error",
      // 無意味な void 演算子を禁止
      "typescript/no-meaningless-void-operator": "error",
      // Promise の誤用を禁止
      // Preact の onClick 等のイベントハンドラで async 関数を渡すパターンが
      // 一般的であり、void 返却の型不一致を許容するため無効化
      "typescript/no-misused-promises": "off",
      // 不適切なスプレッドを禁止
      "typescript/no-misused-spread": "error",
      // 異なる型の enum 混在を禁止
      "typescript/no-mixed-enums": "error",
      // 冗長な型構成要素を禁止
      "typescript/no-redundant-type-constituents": "error",

      // ===== typescript: 不要なコードの検出 =====
      // 不要な boolean リテラル比較を禁止
      "typescript/no-unnecessary-boolean-literal-compare": "error",
      // 不要なテンプレート式を禁止
      "typescript/no-unnecessary-template-expression": "error",
      // 不要な型引数を禁止
      "typescript/no-unnecessary-type-arguments": "error",
      // 不要な型アサーションを禁止
      "typescript/no-unnecessary-type-assertion": "error",

      // ===== typescript: 型安全性 =====
      // 安全でない enum 比較を禁止
      "typescript/no-unsafe-enum-comparison": "error",
      // 安全でない単項マイナスを禁止
      "typescript/no-unsafe-unary-minus": "error",
      // 不要な条件式を禁止 (型情報からデッドブランチを検出)
      "typescript/no-unnecessary-condition": "error",
      // 不要な型変換を禁止 (例: String(stringValue))
      "typescript/no-unnecessary-type-conversion": "error",
      // 不要な型パラメータを禁止 (1 度しか使わない T を検出)
      "typescript/no-unnecessary-type-parameters": "error",

      // ===== typescript: モダン構文の推奨 =====
      // オプショナルチェーンを推奨
      "typescript/prefer-optional-chain": "error",
      // 非 null アサーションのスタイル統一
      "typescript/non-nullable-type-assertion-style": "error",
      // Error オブジェクトのみを throw
      "typescript/only-throw-error": "error",
      // indexOf より includes を推奨
      "typescript/prefer-includes": "error",
      // || より ?? を推奨
      "typescript/prefer-nullish-coalescing": "error",
      // Promise.reject で Error オブジェクトを使用
      "typescript/prefer-promise-reject-errors": "error",
      // reduce の型パラメータを推奨
      "typescript/prefer-reduce-type-parameter": "error",
      // this 型の return を推奨
      "typescript/prefer-return-this-type": "error",
      // Promise を返す関数は async に
      "typescript/promise-function-async": "error",
      // getter/setter の型を一致させる
      "typescript/related-getter-setter-pairs": "error",
      // sort() で比較関数を必須
      "typescript/require-array-sort-compare": "error",
      // async 関数で await を必須
      // インターフェースの統一や将来の await 追加を見越して
      // async 宣言のみの関数を許容するため無効化
      "typescript/require-await": "off",
      // + 演算子のオペランドを制限
      "typescript/restrict-plus-operands": "error",
      // テンプレート式のオペランドを制限
      "typescript/restrict-template-expressions": "error",
      // async 関数で return await を強制
      "typescript/return-await": "error",
      // 厳密な boolean 式を強制
      // Preact signals の .value の truthy チェックや
      // 配列・文字列の存在チェックが頻出するため無効化
      "typescript/strict-boolean-expressions": "off",
      // switch 文で全ケースを網羅
      "typescript/switch-exhaustiveness-check": "error",
      // メソッドの this バインドを強制
      "typescript/unbound-method": "error",
      // catch コールバックで unknown 型を使用
      // Promise チェーンの .catch() コールバック引数に unknown を強制するのは
      // 既存コードの大規模な書き換えが必要で実用的でないため無効化
      "typescript/use-unknown-in-catch-callback-variable": "off",

      // ===== typescript: enum =====
      // enum の重複値を禁止
      "typescript/no-duplicate-enum-values": "error",

      // ===== typescript: null/undefined =====
      // 余分な非 null アサーションを禁止
      "typescript/no-extra-non-null-assertion": "error",
      // オプショナルチェーン後の非 null アサーションを禁止
      "typescript/no-non-null-asserted-optional-chain": "error",

      // ===== typescript: その他 =====
      // this のエイリアスを禁止
      "typescript/no-this-alias": "error",
      // as const を推奨
      "typescript/prefer-as-const": "error",
      // for-of を推奨
      "typescript/prefer-for-of": "error",
      // 関数型を推奨
      "typescript/prefer-function-type": "error",
      // enum メンバーにリテラル値を推奨
      "typescript/prefer-literal-enum-member": "error",

      // ===== typescript: スタイル =====
      // Record 型を推奨
      "typescript/consistent-indexed-object-style": ["error", "record"],
      // interface を推奨
      "typescript/consistent-type-definitions": ["error", "interface"],
      // import type を強制
      "typescript/consistent-type-imports": "error",
      // export type を強制 (consistent-type-imports と対称)
      "typescript/consistent-type-exports": "error",
      // ブラケット記法より dot 記法を推奨
      "typescript/dot-notation": "error",
      // 配列型のスタイル (シンプルな型は T[], 複雑な型は Array<T>)
      "typescript/array-type": [
        "error",
        {
          default: "array-simple",
        },
      ],
      // 「@ts-expect-error」に説明を必須
      "typescript/ban-ts-comment": [
        "error",
        {
          "ts-expect-error": "allow-with-description",
        },
      ],
      // tslint コメントを禁止
      "typescript/ban-tslint-comment": "error",
      // 関数の戻り値型を明示
      // Preact コンポーネントの戻り値型は JSX.Element が自明であり、
      // イベントハンドラやユーティリティ関数も型推論で十分なため無効化
      "typescript/explicit-function-return-type": "off",
      // 紛らわしい非 null アサーションを禁止
      "typescript/no-confusing-non-null-assertion": "error",
      // 動的 delete を禁止
      "typescript/no-dynamic-delete": "error",
      // 空のオブジェクト型を禁止
      "typescript/no-empty-object-type": "error",
      // 不要なクラスを禁止
      "typescript/no-extraneous-class": "error",
      // import type の副作用を禁止
      "typescript/no-import-type-side-effects": "error",
      // 推論可能な型の明示を禁止
      "typescript/no-inferrable-types": "error",
      // 無効な void 型を禁止
      "typescript/no-invalid-void-type": "error",
      // namespace を禁止
      "typescript/no-namespace": "error",
      // require() を禁止
      "typescript/no-require-imports": "error",
      // 不要な空 export を禁止
      "typescript/no-useless-empty-export": "error",
      // ラッパーオブジェクト型を禁止
      "typescript/no-wrapper-object-types": "error",
      // 安全でない宣言マージを禁止
      "typescript/no-unsafe-declaration-merging": "error",
      // 安全でない Function 型を禁止
      "typescript/no-unsafe-function-type": "error",
      // enum 初期化子を推奨
      "typescript/prefer-enum-initializers": "error",
      // namespace キーワードを推奨
      "typescript/prefer-namespace-keyword": "error",
      // トリプルスラッシュ参照を禁止
      "typescript/triple-slash-reference": "error",
      // オーバーロードシグネチャを隣接配置
      "typescript/adjacent-overload-signatures": "error",
      // ジェネリクスコンストラクタのスタイル統一
      "typescript/consistent-generic-constructors": "error",
      // 不要な型制約を禁止
      "typescript/no-unnecessary-type-constraint": "error",

      // ===== oxc: バグ検出 =====
      // Math.PI などの近似定数を検出
      "oxc/approx-constant": "error",
      // arguments への配列メソッド適用を禁止
      "oxc/bad-array-method-on-arguments": "error",
      // 誤ったビット演算を検出
      "oxc/bad-bitwise-operator": "error",
      // charAt() の誤った比較を検出
      "oxc/bad-char-at-comparison": "error",
      // 誤った比較シーケンスを検出
      "oxc/bad-comparison-sequence": "error",
      // Math.min/max の誤用を検出
      "oxc/bad-min-max-func": "error",
      // オブジェクトリテラルの誤った比較を検出
      "oxc/bad-object-literal-comparison": "error",
      // replaceAll の誤った引数を検出
      "oxc/bad-replace-all-arg": "error",
      // 定数比較の矛盾を検出
      "oxc/const-comparisons": "error",
      // 二重比較を検出
      "oxc/double-comparisons": "error",
      // 消去演算 (x * 0) を検出
      "oxc/erasing-op": "error",
      // 誤ったリファクタリングによる代入を検出
      "oxc/misrefactored-assign-op": "error",
      // throw の欠落を検出
      "oxc/missing-throw": "error",
      // ループ内スプレッドの蓄積を禁止
      "oxc/no-accumulating-spread": "error",
      // 範囲外の数値引数を検出
      "oxc/number-arg-out-of-range": "error",
      // 再帰でのみ使用される引数を検出
      "oxc/only-used-in-recursion": "error",
      // 未呼び出しの配列コールバックを検出
      "oxc/uninvoked-array-callback": "error",
      // Map のスプレッドを禁止 (パフォーマンス)
      "oxc/no-map-spread": "error",

      // ===== unicorn: エラー処理 =====
      // catch の error 変数名を統一
      "unicorn/catch-error-name": "error",
      // 空配列スプレッドの一貫性
      "unicorn/consistent-empty-array-spread": "error",
      // 存在チェックのインデックス一貫性
      "unicorn/consistent-existence-index-check": "error",
      // 関数スコープの一貫性
      "unicorn/consistent-function-scoping": "error",
      // Date クローンの一貫性
      "unicorn/consistent-date-clone": "error",
      // Error メッセージを必須
      "unicorn/error-message": "error",

      // ===== unicorn: コードスタイル =====
      // エスケープシーケンスの大文字化
      "unicorn/escape-case": "error",
      // 明示的な length チェック
      "unicorn/explicit-length-check": "error",
      // ビルトインの new 使用を統一
      "unicorn/new-for-builtins": "error",

      // ===== unicorn: 禁止パターン =====
      // eslint-disable の乱用を禁止
      "unicorn/no-abusive-eslint-disable": "error",
      // アクセサの再帰を禁止
      "unicorn/no-accessor-recursion": "error",
      // 配列コールバック参照を禁止
      "unicorn/no-array-callback-reference": "error",
      // forEach を禁止 (for-of を使用)
      "unicorn/no-array-for-each": "error",
      // 配列メソッドの this 引数を禁止
      "unicorn/no-array-method-this-argument": "error",
      // reduce を禁止 (可読性のため)
      "unicorn/no-array-reduce": "error",
      // await 式のメンバーアクセスを禁止
      "unicorn/no-await-expression-member": "error",
      // Promise メソッド内の await を禁止
      "unicorn/no-await-in-promise-methods": "error",
      // 空ファイルを禁止
      "unicorn/no-empty-file": "error",
      // 16 進エスケープを禁止
      "unicorn/no-hex-escape": "error",
      // 即座の変更を禁止
      "unicorn/no-immediate-mutation": "error",
      // ビルトインの instanceof を禁止 (Array を含むため no-instanceof-array は不要)
      "unicorn/no-instanceof-builtins": "error",
      // 無効な fetch オプションを禁止
      "unicorn/no-invalid-fetch-options": "error",
      // slice の終端に length を禁止
      "unicorn/no-length-as-slice-end": "error",
      // 孤立した if を禁止
      "unicorn/no-lonely-if": "error",
      // マジックナンバーの flat 深度を禁止
      "unicorn/no-magic-array-flat-depth": "error",
      // 等価チェックでの否定を禁止
      "unicorn/no-negation-in-equality-check": "error",
      // ネストした三項演算子を禁止
      "unicorn/no-nested-ternary": "error",
      // new Array() を禁止
      "unicorn/no-new-array": "error",
      // new Buffer() を禁止
      "unicorn/no-new-buffer": "error",
      // null を許可 (undefined との使い分けが必要なため)
      "unicorn/no-null": "off",
      // デフォルト引数にオブジェクトを禁止
      "unicorn/no-object-as-default-parameter": "error",
      // 単一 Promise の Promise メソッドを禁止
      "unicorn/no-single-promise-in-promise-methods": "error",
      // static のみのクラスを禁止
      "unicorn/no-static-only-class": "error",
      // thenable オブジェクトを禁止
      "unicorn/no-thenable": "error",
      // this の代入を禁止
      "unicorn/no-this-assignment": "error",
      // typeof undefined を禁止
      "unicorn/no-typeof-undefined": "error",
      // 不要な await を禁止
      "unicorn/no-unnecessary-await": "error",
      // 不要な slice 終端を禁止
      "unicorn/no-unnecessary-slice-end": "error",
      // 読みづらい配列分割代入を禁止
      "unicorn/no-unreadable-array-destructuring": "error",
      // 読みづらい IIFE を禁止
      "unicorn/no-unreadable-iife": "error",
      // 不要なスプレッドフォールバックを禁止
      "unicorn/no-useless-fallback-in-spread": "error",
      // 不要な length チェックを禁止
      "unicorn/no-useless-length-check": "error",
      // 不要な Promise.resolve/reject を禁止
      "unicorn/no-useless-promise-resolve-reject": "error",
      // 不要なスプレッドを禁止
      "unicorn/no-useless-spread": "error",
      // 不要な switch case を禁止
      "unicorn/no-useless-switch-case": "error",
      // 不要な undefined を禁止
      "unicorn/no-useless-undefined": "error",
      // 不要な小数部を禁止 (1.0 → 1)
      "unicorn/no-zero-fractions": "error",

      // ===== unicorn: 数値リテラル =====
      // 数値リテラルの大文字小文字を統一
      "unicorn/number-literal-case": "error",
      // 数値区切りのスタイルを統一
      "unicorn/numeric-separators-style": "error",

      // ===== unicorn: モダン API の推奨 =====
      // find を推奨
      "unicorn/prefer-array-find": "error",
      // flatMap を推奨
      "unicorn/prefer-array-flat-map": "error",
      // flat を推奨
      "unicorn/prefer-array-flat": "error",
      // indexOf を推奨
      "unicorn/prefer-array-index-of": "error",
      // some を推奨
      "unicorn/prefer-array-some": "error",
      // at() を推奨
      "unicorn/prefer-at": "error",
      // codePointAt を推奨
      "unicorn/prefer-code-point": "error",
      // Date.now() を推奨
      "unicorn/prefer-date-now": "error",
      // デフォルトパラメータを推奨
      "unicorn/prefer-default-parameters": "error",
      // globalThis を推奨
      "unicorn/prefer-global-this": "error",
      // 論理演算子を三項演算子より推奨
      "unicorn/prefer-logical-operator-over-ternary": "error",
      // Math.min/max を推奨
      "unicorn/prefer-math-min-max": "error",
      // Math.trunc を推奨
      "unicorn/prefer-math-trunc": "error",
      // モダンな Math API を推奨
      "unicorn/prefer-modern-math-apis": "error",
      // ネイティブ型変換関数を推奨
      "unicorn/prefer-native-coercion-functions": "error",
      // 負のインデックスを推奨
      "unicorn/prefer-negative-index": "error",
      // Number プロパティを推奨
      "unicorn/prefer-number-properties": "error",
      // Object.fromEntries を推奨
      "unicorn/prefer-object-from-entries": "error",
      // オプショナル catch バインディングを推奨
      "unicorn/prefer-optional-catch-binding": "error",
      // プロトタイプメソッドを推奨
      "unicorn/prefer-prototype-methods": "error",
      // Reflect.apply を推奨
      "unicorn/prefer-reflect-apply": "error",
      // RegExp.test を推奨
      "unicorn/prefer-regexp-test": "error",
      // Set.has を推奨
      "unicorn/prefer-set-has": "error",
      // Set.size を推奨
      "unicorn/prefer-set-size": "error",
      // スプレッド構文を推奨
      "unicorn/prefer-spread": "error",
      // 三項演算子より if/else を推奨（多行の場合は三項演算子より可読性が高い）
      "unicorn/prefer-ternary": "off",
      // String.raw を推奨
      "unicorn/prefer-string-raw": "error",
      // replaceAll を推奨
      "unicorn/prefer-string-replace-all": "error",
      // slice を推奨
      "unicorn/prefer-string-slice": "error",
      // startsWith/endsWith を推奨
      "unicorn/prefer-string-starts-ends-with": "error",
      // trimStart/trimEnd を推奨
      "unicorn/prefer-string-trim-start-end": "error",
      // structuredClone を推奨
      "unicorn/prefer-structured-clone": "error",
      // トップレベル await を推奨
      "unicorn/prefer-top-level-await": "error",
      // TypeError を推奨
      "unicorn/prefer-type-error": "error",
      // Blob.text() / Blob.arrayBuffer() を推奨 (OPFS の File 読み出し用)
      "unicorn/prefer-blob-reading-methods": "error",
      // addEventListener を推奨 (onclick= 等の禁止)
      "unicorn/prefer-add-event-listener": "error",
      // appendChild より append を推奨
      "unicorn/prefer-dom-node-append": "error",
      // dataset プロパティを推奨
      "unicorn/prefer-dom-node-dataset": "error",
      // remove() を推奨 (parentNode.removeChild より)
      "unicorn/prefer-dom-node-remove": "error",
      // textContent を推奨 (innerText より)
      "unicorn/prefer-dom-node-text-content": "error",
      // querySelector を推奨 (getElementById 等より)
      "unicorn/prefer-query-selector": "error",
      // KeyboardEvent.key を推奨 (keyCode 等より)
      "unicorn/prefer-keyboard-event-key": "error",
      // import.meta.dirname / filename を推奨
      "unicorn/prefer-import-meta-properties": "error",
      // document.cookie の使用を禁止 (セキュリティ)
      "unicorn/no-document-cookie": "error",
      // Error.captureStackTrace の不要呼び出しを検出
      "unicorn/no-useless-error-capture-stack-trace": "error",

      // ===== unicorn: 必須引数 =====
      // join の区切り文字を必須
      "unicorn/require-array-join-separator": "error",
      // モジュール属性を必須
      "unicorn/require-module-attributes": "error",
      // toFixed の桁数を必須
      "unicorn/require-number-to-fixed-digits-argument": "error",

      // ===== unicorn: スタイル =====
      // switch case の波括弧を統一
      "unicorn/switch-case-braces": "error",
      // テキストエンコーディング識別子の大文字小文字を統一
      "unicorn/text-encoding-identifier-case": "error",
      // new Error() を強制
      "unicorn/throw-new-error": "error",
      // assert の一貫性
      "unicorn/consistent-assert": "error",
      // クラスフィールドを推奨
      "unicorn/prefer-class-fields": "error",
      // 匿名 default export を禁止
      "unicorn/no-anonymous-default-export": "error",

      // ===== import: 正確性 =====
      // default import の存在確認
      "import/default": "error",
      // export の整合性確認
      "import/export": "error",
      // import を先頭に配置
      "import/first": "error",
      // named import の存在確認
      "import/named": "error",
      // namespace の存在確認
      "import/namespace": "error",

      // ===== import: 禁止パターン =====
      // 絶対パスの import を禁止
      "import/no-absolute-path": "error",
      // AMD を禁止
      "import/no-amd": "error",
      // 循環依存を禁止
      "import/no-cycle": "error",
      // 重複 import を禁止
      "import/no-duplicates": "error",
      // 空の名前付きブロックを禁止
      "import/no-empty-named-blocks": "error",
      // ミュータブルな export を禁止
      "import/no-mutable-exports": "error",
      // default と同名の named export を禁止
      "import/no-named-as-default": "error",
      // default のメンバーアクセスを禁止
      "import/no-named-as-default-member": "error",
      // named として default を import することを禁止
      "import/no-named-default": "error",
      // 自己 import を禁止
      "import/no-self-import": "error",
      // webpack ローダー構文を禁止
      "import/no-webpack-loader-syntax": "error",

      // ===== promise: 必須パターン =====
      // always return を強制
      "promise/always-return": "error",
      // new Promise を許可 (ストリーム処理等で必要)
      "promise/avoid-new": "off",
      // catch または return を強制
      "promise/catch-or-return": "error",

      // ===== promise: 禁止パターン =====
      // Promise 内のコールバックを禁止
      "promise/no-callback-in-promise": "error",
      // 複数回の resolve/reject を禁止
      "promise/no-multiple-resolved": "error",
      // Promise のネストを禁止
      "promise/no-nesting": "error",
      // Promise の静的メソッドへの new を禁止
      "promise/no-new-statics": "error",
      // コールバック内の Promise を禁止
      "promise/no-promise-in-callback": "error",
      // finally での return を禁止
      "promise/no-return-in-finally": "error",
      // 不要な Promise ラップを禁止
      "promise/no-return-wrap": "error",
      // パラメータ名を統一
      "promise/param-names": "error",

      // ===== promise: モダン構文の推奨 =====
      // コールバックより await を推奨
      "promise/prefer-await-to-callbacks": "error",
      // then より await を推奨
      "promise/prefer-await-to-then": "error",
      // catch メソッドを推奨
      "promise/prefer-catch": "error",
      // 有効なパラメータを強制
      "promise/valid-params": "error",

      // ===== vitest: テストの品質 =====
      // TODO コメントの警告
      "vitest/warn-todo": "error",
      // vi/vitest の一貫した使用
      "vitest/consistent-vitest-vi": "error",
      // each/for の一貫性
      "vitest/consistent-each-for": "error",
      // ホイスト API をファイル先頭に配置
      "vitest/hoisted-apis-on-top": "error",
      // 不要な async expect 関数を禁止
      "vitest/no-unneeded-async-expect-function": "error",
      // 呼び出し回数の検証を推奨
      "vitest/prefer-called-times": "error",
      // toHaveBeenCalledOnce を推奨
      "vitest/prefer-called-once": "error",
      // spy を推奨
      // モックやスタブを利用しないポリシー (CLAUDE.md) のため無効化
      "vitest/prefer-spy-on": "off",
      // モック promise のショートハンド (mockResolvedValue 等) を推奨
      // モックやスタブを利用しないポリシー (CLAUDE.md) のため無効化
      "vitest/prefer-mock-promise-shorthand": "off",
      // モック return のショートハンド (mockReturnValue 等) を推奨
      // モックやスタブを利用しないポリシー (CLAUDE.md) のため無効化
      "vitest/prefer-mock-return-shorthand": "off",
      // vi.fn / vi.spyOn / vi.mock の型パラメータを必須
      // モックやスタブを利用しないポリシー (CLAUDE.md) のため無効化
      "vitest/require-mock-type-parameters": "off",
      // テストファイル名の一貫性
      "vitest/consistent-test-filename": "error",
      // require-hook を強制
      "vitest/require-hook": "error",
      // test.only の commit を防止
      "vitest/no-focused-tests": "error",
      // test.skip の commit を防止
      "vitest/no-disabled-tests": "error",
      // if 等の中で test() 宣言を禁止
      "vitest/no-conditional-tests": "error",
      // node:test からの import を禁止 (vitest のテストと混同を防止)
      "vitest/no-import-node-test": "error",
      // test/describe のタイトルが文字列であることを保証
      "vitest/valid-title": "error",
      // test.skip / xtest 等のプレフィックス記法を禁止 (skip()/only() を使用)
      "vitest/no-test-prefixes": "error",
      // test コールバックでの return を禁止 (await を使う)
      "vitest/no-test-return-statement": "error",

      // ===== react: ライフサイクル (Preact 互換) =====
      // componentDidMount 内での setState を禁止
      "react/no-did-mount-set-state": "error",
      // componentWillUpdate 内での setState を禁止
      "react/no-will-update-set-state": "error",
      // unsafe ライフサイクルメソッドを禁止
      "react/no-unsafe": "error",
      // SFC 内での this 使用を禁止
      "react/no-this-in-sfc": "error",

      // ===== react: JSX の正確性 (Preact 互換) =====
      // リストに key プロパティを強制
      "react/jsx-key": "error",
      // 重複 props を禁止
      "react/jsx-no-duplicate-props": "error",
      // 未定義コンポーネントを禁止
      "react/jsx-no-undef": "error",
      // 複数スプレッドを禁止
      "react/jsx-props-no-spread-multi": "error",
      // children を props として渡すことを禁止
      "react/no-children-prop": "error",
      // dangerouslySetInnerHTML と children の併用禁止
      "react/no-danger-with-children": "error",
      // void 要素 (img, br 等) に children を禁止
      "react/void-dom-elements-no-children": "error",

      // ===== react: Hooks (Preact 互換) =====
      // Hooks のルール検証 (条件分岐内での使用禁止等)
      "react/rules-of-hooks": "error",
      // useEffect 等の依存配列の検証
      "react/exhaustive-deps": "error",

      // ===== react: セキュリティ (Preact 互換) =====
      // target="_blank" に rel="noopener noreferrer" を強制
      "react/jsx-no-target-blank": "error",
      // javascript: URL を禁止
      "react/jsx-no-script-url": "error",
      // iframe に sandbox 属性を強制
      "react/iframe-missing-sandbox": "error",
      // dangerouslySetInnerHTML を禁止
      "react/no-danger": "error",

      // ===== react: コード品質 (Preact 互換) =====
      // コメントがテキストノードになることを防止
      "react/jsx-no-comment-textnodes": "error",
      // エスケープされていない文字を禁止
      "react/no-unescaped-entities": "error",
      // style prop にオブジェクトを強制
      "react/style-prop-object": "error",
      // 不要な Fragment を禁止
      "react/jsx-no-useless-fragment": "error",

      // ===== react: スタイル (Preact 互換) =====
      // button に type 属性を強制
      "react/button-has-type": "error",
      // boolean 属性のスタイル統一 (checked={true} → checked)
      "react/jsx-boolean-value": "error",
      // 波括弧の使用統一
      "react/jsx-curly-brace-presence": "error",
      // Fragment のスタイル統一 (<></> vs <Fragment>)
      "react/jsx-fragments": "error",
      // コンポーネント名を PascalCase に
      "react/jsx-pascal-case": "error",
      // 自己閉じタグを推奨 (<div /> vs <div></div>)
      "react/self-closing-comp": "error",

      // ===== react: 不要なルール (Preact では無効化) =====
      // Preact では JSX プラグマを使用するため不要
      "react/react-in-jsx-scope": "off",
      // JSX のネスト深度制限はコンポーネントに厳しすぎる
      "react/jsx-max-depth": "off",
      // props スプレッドは Preact コンポーネントで便利
      "react/jsx-props-no-spreading": "off",

      // ===== eslint: プロジェクトのスタイルに合わない制限ルール =====
      // named export は Preact コンポーネントで標準的
      "import/no-named-export": "off",
      // export のグループ化は不要
      "import/group-exports": "off",
      // 単一 export でも default を強制しない
      "import/prefer-default-export": "off",
      // export を最後に配置する制約は不要
      "import/exports-last": "off",
      // マジックナンバーの制限は厳しすぎる
      "no-magic-numbers": "off",
      // function 宣言は Preact コンポーネントで標準的
      "func-style": "off",
      // 三項演算子は可読性が高い場合に有用
      "no-ternary": "off",
      // 日本語コメントに大文字開始ルールは不適
      "capitalized-comments": "off",
      // PascalCase のコンポーネントファイル名が標準
      "unicorn/filename-case": "off",
      // 短い変数名が必要な場合がある
      "id-length": "off",
      // コンポーネントの行数制限は厳しすぎる
      "max-lines-per-function": "off",
      // オブジェクトのキーは意味順に並べたい
      "sort-keys": "off",
      // インラインコメントは有用
      "no-inline-comments": "off",
      // 変数の初期化タイミングは柔軟に
      "init-declarations": "off",
      // ファイル行数の制限は厳しすぎる
      "max-lines": "off",
      // 引数の数の制限は厳しすぎる
      "max-params": "off",

      // ===== eslint: pedantic カテゴリから無効化 =====
      // async 関数で await を必須 (eslint 版)
      // typescript/require-await と同様の理由で無効化
      "require-await": "off",
      // no-useless-undefined が return undefined を return に自動修正することで
      // typescript/consistent-return と衝突するため無効化する
      "typescript/consistent-return": "off",

      // ===== eslint: プロジェクトに不適な制限ルール =====
      // import の並び順は oxfmt で管理
      "sort-imports": "off",
      // 否定条件は可読性に問題ない場合が多い
      "no-negated-condition": "off",
      "unicorn/no-negated-condition": "off",
      // 順次処理が必要な場合がある
      "no-await-in-loop": "off",
      // コールバック内の変数シャドウは一般的
      "no-shadow": "off",
      // ネストした三項演算子は場合による
      "no-nested-ternary": "off",
      // type import と value import の重複は許容
      "no-duplicate-imports": "off",
      // TODO コメントは残したい
      "no-warning-comments": "off",
      // continue は有用
      "no-continue": "off",

      // ===== import: プロジェクトに不適なルール =====
      // 依存数制限は厳しすぎる
      "import/max-dependencies": "off",
      // Node.js モジュールは vite.config 等で必要
      "import/no-nodejs-modules": "off",
      // CSS import 等で副作用 import が必要
      "import/no-unassigned-import": "off",
      // namespace import は便利
      "import/no-namespace": "off",

      // ===== unicorn: プロジェクトに不適なルール =====
      // postMessage の target origin は WebRTC で不要な場合がある
      "unicorn/require-post-message-target-origin": "off",

      // ===== vitest/jest: プロジェクトに不適なルール =====
      // vite-plus/test や @playwright/test 経由で import しており globals は不要
      "vitest/no-importing-vitest-globals": "off",
      "vitest/prefer-importing-vitest-globals": "off",
      // beforeEach/afterEach 等のフックは Vitest で標準的に利用するため無効化
      "vitest/no-hooks": "off",
      // top-level describe の強制は不要
      "vitest/require-top-level-describe": "off",
      // テストタイトルの lower case 強制は不要
      "vitest/prefer-lowercase-title": "off",
      // テストは Chai API の assert を利用しており expect() を呼ばないため無効化
      "vitest/expect-expect": "off",
      // Chai assert 利用のため expect.hasAssertions() は不要
      "vitest/prefer-expect-assertions": "off",
      // jest ルールはプロジェクトで使用しない
      "jest/require-hook": "off",
      "jest/require-top-level-describe": "off",
      "jest/prefer-lowercase-title": "off",
      "jest/no-hooks": "off",
      "jest/prefer-ending-with-an-expect": "off",

      // ===== typescript: 採用しない pedantic 系ルール =====
      // 全関数パラメータに readonly を要求するルール。 Preact プロップスや
      // sora-js-sdk の MediaTrackConstraints など外部 API の型と相性が悪く
      // 全面採用が困難なため無効化する
      "typescript/prefer-readonly-parameter-types": "off",
      // JSON.parse / DOM event.target など `as Type` の限定的な使用は
      // 型ガードを毎回挟むより読みやすく、 sora-js-sdk が unknown を返す API も
      // あるため本体では無効化する (テストファイルは override で off)
      "typescript/no-unsafe-type-assertion": "off",
      // Preact の onClick 等イベントハンドラに value-returning 関数や
      // async 関数を渡すパターンが一般的であり no-misused-promises と同じ理由で無効化
      "typescript/strict-void-return": "off",
    },
    overrides: [
      {
        // アプリケーションエントリポイントは Vitest の hook ルール対象外
        files: ["src/main.tsx"],
        rules: {
          "vitest/require-hook": "off",
        },
      },
      {
        // テストファイルは型安全性を緩和
        files: ["**/*.test.ts", "**/*.prop.ts"],
        rules: {
          "typescript/no-explicit-any": "off",
          "typescript/no-non-null-assertion": "off",
          "typescript/no-unsafe-argument": "off",
          "typescript/no-unsafe-assignment": "off",
          "typescript/no-unsafe-call": "off",
          "typescript/no-unsafe-member-access": "off",
          "typescript/no-unsafe-return": "off",
          "typescript/no-unsafe-type-assertion": "off",
        },
      },
    ],
    ignorePatterns: ["dist/**", "node_modules/**", ".pnpmfile.cjs"],
  },
  fmt: {
    ignorePatterns: ["dist/**"],
  },
});
