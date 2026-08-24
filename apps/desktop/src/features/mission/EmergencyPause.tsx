export function EmergencyPause({ disabled = false, onPause }: { disabled?: boolean; onPause: () => void }) {
  return <button className="emergency-pause" type="button" disabled={disabled} onClick={onPause} aria-label="Request safe pause">Pause mission</button>;
}
