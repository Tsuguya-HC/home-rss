import { useState } from 'react'

interface AddFeedModalProps {
  onAdd: (url: string) => Promise<void>
  onClose: () => void
}

export function AddFeedModal({ onAdd, onClose }: AddFeedModalProps) {
  const [url, setUrl] = useState('')
  const [loading, setLoading] = useState(false)

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!url.trim()) return
    setLoading(true)
    try {
      await onAdd(url.trim())
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2 className="modal-title">フィード追加</h2>
        <form onSubmit={handleSubmit}>
          <input
            className="modal-input"
            type="url"
            placeholder="https://example.com/feed.xml"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            autoFocus
            required
          />
          <div className="modal-actions">
            <button type="button" className="btn btn--secondary" onClick={onClose}>
              キャンセル
            </button>
            <button type="submit" className="btn btn--primary" disabled={loading}>
              {loading ? '追加中...' : '追加'}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}
