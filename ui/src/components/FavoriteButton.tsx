import type { MouseEvent } from 'react'

interface FavoriteButtonProps {
  favorite: boolean
  pending: boolean
  label: string
  onToggle: (e: MouseEvent<HTMLButtonElement>) => void
}

export function FavoriteButton({ favorite, pending, label, onToggle }: FavoriteButtonProps) {
  return (
    <button
      type="button"
      className={`favorite-star${favorite ? ' favorite-star--active' : ''}`}
      aria-label={favorite ? 'お気に入りを解除' : 'お気に入りに追加'}
      aria-pressed={favorite}
      disabled={pending}
      onClick={onToggle}
    >
      {label}
    </button>
  )
}
