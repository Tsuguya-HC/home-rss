# home-rss

Spin (WebAssembly) で構築するカスタム RSS リーダー。SpinKube 経由で Kubernetes (Talos Linux) にデプロイする。

## アーキテクチャ

- **Runtime**: Spin (WebAssembly) on SpinKube
- **言語**: Rust
- **DB**: PostgreSQL (CNPG shared-pg)
- **認証**: Kanidm OIDC via oauth2-proxy
- **デプロイ**: ArgoCD GitOps

## フェーズ

| Phase | 内容 | 状態 |
|-------|------|------|
| 1 | SpinKube インフラ (Talos 拡張 + spin-operator) | 未着手 |
| 2 | RSS リーダーアプリ (Rust + Spin) | 未着手 |
| 3 | K8s インテグレーション (SSO, CNP, Gateway) | 未着手 |
