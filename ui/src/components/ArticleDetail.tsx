import DOMPurify from 'dompurify'
import { Article } from '../types'
import { formatDateTime } from '../lib/date'

interface ArticleDetailProps {
  article: Article
  onBack: () => void
}

export function ArticleDetail({ article, onBack }: ArticleDetailProps) {
  const date = article.published_at ? formatDateTime(article.published_at) : null

  return (
    <div className="detail-inner">
      <div className="detail-header">
        <button className="btn-icon back-btn mobile-back-btn" onClick={onBack} title="戻る">
          ←
        </button>
        <a
          href={article.url}
          target="_blank"
          rel="noopener noreferrer"
          className="detail-link"
        >
          元記事を開く ↗
        </a>
      </div>
      <article className="detail-article">
        <h1 className="detail-title">{article.title}</h1>
        <div className="detail-meta">
          {article.author && <span>{article.author}</span>}
          {date && <span>{date}</span>}
        </div>
        {article.content ? (
          <div
            className="detail-content"
            dangerouslySetInnerHTML={{ __html: DOMPurify.sanitize(article.content) }}
          />
        ) : (
          <div className="detail-no-content">
            <p>本文がありません。</p>
            <a href={article.url} target="_blank" rel="noopener noreferrer">
              元記事を読む →
            </a>
          </div>
        )}
      </article>
    </div>
  )
}
