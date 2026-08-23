# home-rss

Spin (WebAssembly) で構築するカスタム RSS リーダー。SpinKube 経由で Kubernetes (Talos Linux) にデプロイする。

## アーキテクチャ

- **Runtime**: Spin 4.x (WebAssembly) on SpinKube (containerd-shim-spin v0.25.1+)
- **言語**: Rust → wasm32-wasip1 (spin-sdk 6.x / WASI 0.3)
- **DB**: PostgreSQL (CNPG `rss-pg`) — TLS 必須、CA は変数 `db_ca_root` で注入
- **認証**: Kanidm OIDC via oauth2-proxy
- **デプロイ**: ArgoCD GitOps

## サービス構成

| サービス | trigger | 役割 | K8s リソース |
|---------|---------|------|-------------|
| server | http | REST API | SpinApp |
| ui | http | Web UI (HTMX) | SpinApp |
| fetcher | command | フィード収集 | CronJob |
| cleaner | command | 古い記事削除 | CronJob |

## API エンドポイント

`server` (REST API) が提供するエンドポイント一覧。実装は `server/src/lib.rs` の `route()`。

| メソッド | パス | 説明 |
|---------|------|------|
| GET | `/api/feeds` | フィード一覧を取得 |
| POST | `/api/feeds` | フィードを追加（body: `{"url": string}`）。URL が既存の場合は既存レコードを返す |
| DELETE | `/api/feeds/:id` | フィードを削除 |
| GET | `/api/articles` | 記事一覧を取得。クエリパラメータ `feed_id`（フィードで絞り込み）、`unread=true`（未読のみ）に対応 |
| POST | `/api/articles/:id/read` | 記事を既読にする |
| POST | `/api/articles/read-all` | 全記事を既読にする |
| POST | `/api/import/opml` | OPML ファイルをインポートし、含まれるフィードを一括登録（body: OPML XML） |
| GET | `/api/stats` | フィード数・未読記事数を取得 |

## 開発

```bash
# ビルド（全サービス）
cargo build --target wasm32-wasip1 --release

# 個別サービスのビルド + 起動
cd server && spin build && spin up
cd ui && spin build && spin up
cd fetcher && spin build && spin up  # 即時実行して終了
```
