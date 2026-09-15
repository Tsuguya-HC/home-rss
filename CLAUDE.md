# home-rss

Spin (WebAssembly) で構築するカスタム RSS リーダー。SpinKube 経由で home-cluster (Talos Linux K8s) にデプロイする。

## アーキテクチャ

```
                   ┌─ CronJob (15-30min) ─── fetcher (command trigger) ──┐
                   │                                  │                   │
                   │                            外部フィード (HTTPS)      │
                   │                                  │                   │
rss.infra.tgy.io → oauth2-proxy → server (http trigger) ─── API ────────├── shared-pg
                                   ui (http trigger) ─── 静的ファイル     │
                   └─ CronJob (daily) ──── cleaner (command trigger) ────┘
```

**server と fetcher の両方が外部フィードを取りに行く。** フィード追加 (`POST /api/feeds`) は
その場で 1 回取得し、以降は fetcher が定期取得する。取得+保存の実体は
`shared/src/fetch.rs` の `fetch_and_store()` ひとつで、違いはタイムアウトの有無だけ
（server は有界、fetcher は打ち切りなし）。**二重実装にしないこと** — 分かれていた頃は
ETag / Last-Modified の扱いが既に乖離していた。

ユーザーが指定した URL をサーバーが取りに行くので、`shared/src/ssrf.rs` の
`reject_internal_feed_url()` が `fetch_and_store()` の入口で内部宛を弾く（追加時の 1 回だけでなく
**定期取得のたびに効く**）。ネットワーク層の境界は
[docs/network-access.md](docs/network-access.md) を参照。

| サービス | trigger | 役割 | K8s リソース | OCI イメージ |
|---------|---------|------|-------------|-------------|
| server | http | REST API + フィード追加時の即時取得 | SpinApp | `ghcr.io/tsuguya-hc/home-rss-server` |
| ui | http | Web UI (React SPA) + 静的ファイル (spin-fileserver) | SpinApp | `ghcr.io/tsuguya-hc/home-rss-ui` |
| fetcher | command | フィード収集 → DB 書き込み | CronJob | `ghcr.io/tsuguya-hc/home-rss-fetcher` |
| cleaner | command | 古い記事の削除 | CronJob | `ghcr.io/tsuguya-hc/home-rss-cleaner` |

## リポジトリ構成

```
home-rss/
├── server/           # REST API (http trigger)
├── ui/               # Web UI (React + Vite + TypeScript, pnpm)。ビルド成果物を spin-fileserver が配信
├── fetcher/          # フィード収集 (command trigger)
├── cleaner/          # 古い記事削除 (command trigger)
├── shared/           # 共有ライブラリ (DB モデル、型定義、取得+保存、SSRF ガード)
├── migrations/       # SQL マイグレーション
├── docs/             # 運用メモ（network-access.md: CNP と外部アクセス）
├── Cargo.toml        # workspace
└── .github/workflows/
    ├── ci.yml        # detect changes → leaf ジョブ → CI Gate
    ├── _build-rust.yml / _build-ui.yml / _test.yml   # leaf（reusable）
    └── release.yml   # イメージを GHCR push
```

Cargo workspace で `shared` クレートを共有。各サービスは独立した Spin app (spin.toml + Cargo.toml)。

## 技術スタック

- **言語**: Rust → wasm32-wasip1
- **フレームワーク**: spin-sdk 6.x (HTTP, PostgreSQL, outbound HTTP) — WASI 0.3 ベースで全 API が async
- **UI**: React + Vite + TypeScript（`ui/`、pnpm 管理）→ ビルド成果物を spin-fileserver が配信
- **DB**: PostgreSQL (CNPG `rss-pg`, `rssreader` DB) — TLS 検証あり
- **認証**: Kanidm OIDC via oauth2-proxy
- **ランタイム**: SpinKube (containerd-shim-spin on Talos)
- **CI**: GitHub Actions (`spin build` → `spin registry push`)

## 開発

### ローカル実行

```bash
cd server && spin build && spin up
cd fetcher && spin build && spin up  # 即時実行して終了
```

### ビルド

```bash
cargo build --target wasm32-wasip1 --release  # or spin build
```

### マイグレーション

[dbmate](https://github.com/amacneil/dbmate) で管理。ファイルは `migrations/` に `YYYYMMDDHHMMSS_name.sql` 形式で配置。

```bash
# 新しいマイグレーション作成
dbmate new add_some_column

# ローカルで実行
dbmate --url "postgres://user:pass@localhost:5432/rssreader?sslmode=disable" up
```

本番では ArgoCD PreSync Hook Job (home-cluster 側) が `dbmate up` を実行する。

### 並列開発

worktree を切って server/ui/fetcher/cleaner を並列セッションで開発可能。
`shared` クレートを先に固めておくこと。

## 関連リポジトリ

- **home-cluster**: K8s マニフェスト (SpinApp, CronJob, CNP, oauth2-proxy, OnePasswordItem)
- **home-infra**: Talos 設定 (containerd-shim-spin 拡張は Talos イメージに組み込み済み)
- **home-cloudflare**: DNS / Tunnel (rss.infra.tgy.io)

## Issue 管理

Issue はフェーズとリポジトリのラベルで分類:
- `phase:infra` — SpinKube インフラ (#1-#3)、作業は主に home-cluster
- `phase:app` — アプリ開発 (#4, #5, #7, #8)、作業は主にこのリポジトリ
- `phase:integration` — K8s デプロイ・SSO・CNP (#6)、作業は主に home-cluster

## 注意事項

- **PUBLIC リポジトリ** — 機密値を絶対にコミットしない
- Spin の cron trigger は SpinKube 非対応 → command trigger + K8s CronJob を使う
- **フィード取得は HTTPS のみ**。`fetcher/spin.toml` / `server/spin.toml` の
  `allowed_outbound_hosts` に `http://*:80` を戻しても、home-cluster の CNP が world:443 しか
  開けていないので平文フィードは失敗ではなくハングする。開けるなら両方を揃えて直すこと。
  外部アクセスの全体像と、新しい宛先が要るときの手順は [docs/network-access.md](docs/network-access.md)
- **`shared` の `feed` feature** が feed パース（feed-rs / url）と `fetch_and_store` を囲う。
  有効化するのは server / fetcher だけで、cleaner は引き込まない。ただし
  `cargo test --workspace` は feature unification で全部に効くので、**単体ビルド
  (`cargo build -p ...`) でしか差は出ない**（`shared/Cargo.toml` に実測メモ）
- Spin SDK の PostgreSQL データ型サポートを事前に確認すること (UUID, TIMESTAMPTZ 等)

### spin-sdk 6.x への移行で踏んだところ

- `pg4` / `pg3` モジュールは廃止され `spin_sdk::pg` に一本化。`variables::get` /
  `Connection::open` / `query` / `execute` は**すべて async**
- `#[http_component]` → `#[http_service]`。**`Router` と `Params` は削除された**ので
  ルーティングは自前（`server/src/lib.rs` の `route()` が method × path セグメントで分岐）
- `Request` / `Response` は hyperium (`http` crate) の型。body は
  `req.into_body().bytes().await?` で読む（`IncomingBodyExt`）
- `QueryResult` はストリーム。`.collect().await?` で `Vec<Row>` を得る
- **パラメータに直接キャストを書くと、PostgreSQL はそのパラメータをその型と推論する。**
  `$1::uuid` に `Str` を渡すと `WrongType { postgres: Uuid, rust: "String" }` で落ちる。
  対処は 2 通り:
  - 対応する variant がある型（UUID）は `ParameterValue::Uuid` を渡し、キャストを外す
  - variant が無い型（TIMESTAMPTZ / INTERVAL）は **`$1::text::timestamptz` のように
    text を経由**する。こう書くと推論は text になる。SDK は `NaiveDateTime` を
    TIMESTAMP にしかマップせず、この DB の時刻列は全て TIMESTAMPTZ
  - 読み取り側の `SELECT id::text` / `EXTRACT(EPOCH FROM ...)` は逆向きなので影響なし

### PostgreSQL TLS

`db_url` の **`sslmode` に `verify-ca` / `verify-full` は指定できない**
（`invalid value for option sslmode` でパースが落ちる）。`require` を使う。

`require` でも Spin は常に完全な証明書検証を行う（実測: 正しい CA → ホスト名検証まで到達、
誤った CA → `self-signed certificate in certificate chain` で拒否）。したがって
**`sslmode=require` + `db_ca_root` で verify-full 相当**になる。

CA は `rss-pg-ca` secret の `ca.crt` を SpinApp 変数 `db_ca_root` に注入する
（`shared/src/db.rs` が `Certificate::Text` として `set_ca_root` に渡す）。
接続先は証明書 SAN に載っている名前を使うこと（`rss-pg-rw.rss.svc.cluster.local`）。
