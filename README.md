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

## 開発

```bash
# ビルド（全サービス）
cargo build --target wasm32-wasip1 --release

# 個別サービスのビルド + 起動
cd server && spin build && spin up
cd ui && spin build && spin up
cd fetcher && spin build && spin up  # 即時実行して終了
```
