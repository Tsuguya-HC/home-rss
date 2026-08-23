import { Article } from '../types'
import { formatDate } from '../lib/date'

interface ArticleListItemProps {
  article: Article
  isSelected: boolean
  onClick: () => void
}

export function ArticleListItem({ article, isSelected, onClick }: ArticleListItemProps) {
  const date = article.published_at ? formatDate(article.published_at) : null

  return (
    <li
      className={`article-item${isSelected ? ' article-item--active' : ''}`}
      onClick={onClick}
    >
      <div className="article-item-title">{article.title}</div>
      {date && <div className="article-item-date">{date}</div>}
    </li>
  )
}
