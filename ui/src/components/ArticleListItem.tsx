import { useEffect, useState } from 'react'
import { Article } from '../types'
import { formatDate } from '../lib/date'

interface ArticleListItemProps {
  article: Article
  isSelected: boolean
  onClick: () => void
}

export function ArticleListItem({ article, isSelected, onClick }: ArticleListItemProps) {
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
        <div className="article-item-title">{article.title}</div>
        {date && <div className="article-item-date">{date}</div>}
      </div>
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
