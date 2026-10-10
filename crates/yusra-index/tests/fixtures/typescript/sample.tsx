import React from "react";

/** Tombol. */
export function Button({ label }: { label: string }) {
  return <button onClick={() => alert(label)}>{label}</button>;
}

export const Card = ({ title }: { title: string }) => (
  <div className="card">{title}</div>
);
