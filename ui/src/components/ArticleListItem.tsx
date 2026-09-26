import { useEffect, useState } from 'react'
import { Article } from '../types'
import { formatDate } from '../lib/date'

interface ArticleListItemProps {
  article: Article
  isSelected: boolean
  onClick: () => void
  onToggleFavorite: () => void
}

export function ArticleListItem({ article, isSelected, onClick, onToggleFavorite }: ArticleListItemProps) {
  const date = article.published_at ? formatDate(article.published_at) : null
  const [imgHidden, setImgHidden] = useState(false)

  useEffect(() => {
    setImgHidden(false)
  }, [article.id, article.image_url])

  const imageUrl = imgHidden ? null : article.image_url

  return (
    <li
      className={`article-item${isSelected ? ' article-item--active' : ''}${article.image_url ? ' article-item--with-thumbnail' : ''}`}
      onClick={onClick}
    >
      <div className="article-item-body">
        <div className="article-item-title">
          {article.is_favorite && <span className="favorite-star">★</span>}
          {article.title}
        </div>
        {date && <div className="article-item-date">{date}</div>}
      </div>
      <button
        className="btn-icon favorite-toggle"
        onClick={(e) => {
          e.stopPropagation()
          onToggleFavorite()
        }}
        title={article.is_favorite ? 'お気に入りを外す' : 'お気に入りにする'}
      >
        {article.is_favorite ? '★' : '☆'}
      </button>
      {imageUrl && (
        <img
          className="article-item-thumbnail"
          src={imageUrl}
          alt=""
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          onError={() => setImgHidden(true)}
        />
      )}
    </li>
  )
}
