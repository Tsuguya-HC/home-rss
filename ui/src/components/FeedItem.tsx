import { useState } from 'react'
import { Feed } from '../types'

interface FeedItemProps {
  feed: Feed
  unreadCount: number
  isSelected: boolean
  onSelect: () => void
  onDelete: () => Promise<void>
}

export function FeedItem({ feed, unreadCount, isSelected, onSelect, onDelete }: FeedItemProps) {
  const [confirmDelete, setConfirmDelete] = useState(false)

  const handleDelete = async (e: React.MouseEvent) => {
    e.stopPropagation()
    if (!confirmDelete) {
      setConfirmDelete(true)
      setTimeout(() => setConfirmDelete(false), 3000)
      return
    }
    await onDelete()
  }

  return (
    <div className={`feed-item${isSelected ? ' feed-item--active' : ''}`} onClick={onSelect}>
      <span className="feed-name">{feed.title || feed.url}</span>
      <span className="feed-actions">
        {unreadCount > 0 && <span className="unread-badge">{unreadCount}</span>}
        <button
          className={`delete-btn${confirmDelete ? ' delete-btn--confirm' : ''}`}
          onClick={handleDelete}
          title={confirmDelete ? '確認: もう一度クリックで削除' : '削除'}
        >
          {confirmDelete ? '!' : '×'}
        </button>
      </span>
    </div>
  )
}
