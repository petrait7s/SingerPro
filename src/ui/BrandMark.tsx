export function PetraMark({ size = 24 }: { size?: number }) {
  return <img src="/petra-mark.png" width={size} height={size} alt="PETRA Software Solutions" />;
}

export function SingerProWordmark({ light = false }: { light?: boolean }) {
  return (
    <span
      className={`metric-display text-[17px] font-bold tracking-tight ${light ? "text-white" : "text-navy"}`}
    >
      Singer<span className="text-teal"> Pro</span>
    </span>
  );
}
