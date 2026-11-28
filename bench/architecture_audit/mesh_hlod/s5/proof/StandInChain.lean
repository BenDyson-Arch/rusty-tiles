import Mathlib

/-! Root recovery v2: the delivered error order through unpublished stand-ins.

An omitted cell passes up a stand-in whose declared error is the largest declared error of
its inputs: a built (published) tile contributes its own G, an omitted child its stand-in's
declared error. The recovered root's children error is that same maximum over its inputs,
and its published G is at least its children error (the producer's check, S2-measured
against the source independently of the stand-ins). The delivered frontier is every built
tile reached through omitted cells. Then the root's G is at least every frontier tile's G:
the order the validator recounts (`recoveryError`). Errors are nonnegative, so the empty
maximum is 0. -/

inductive Cell where
  | built (g : ℝ)
  | omitted (kids : List Cell)

mutual
/-- A cell's contribution to its parent's children error: a built tile's G, or an
omitted cell's stand-in declared error (the largest over its inputs). -/
def Cell.val : Cell → ℝ
  | .built g => g
  | .omitted kids => Cell.vals kids

def Cell.vals : List Cell → ℝ
  | [] => 0
  | c :: cs => max c.val (Cell.vals cs)
end

mutual
/-- The published tiles a cell delivers in its place: itself if built, otherwise the
frontier of its children (the bypassed scaffold contributes none). -/
def Cell.frontier : Cell → List ℝ
  | .built g => [g]
  | .omitted kids => Cell.frontiers kids

def Cell.frontiers : List Cell → List ℝ
  | [] => []
  | c :: cs => c.frontier ++ Cell.frontiers cs
end

mutual
theorem Cell.frontier_le : ∀ (c : Cell), ∀ g ∈ c.frontier, g ≤ c.val
  | .built g', g, h => by simp [Cell.frontier, Cell.val] at h ⊢; exact h.le
  | .omitted kids, g, h => by
      simp only [Cell.frontier, Cell.val] at h ⊢
      exact Cell.frontiers_le kids g h

theorem Cell.frontiers_le : ∀ (cs : List Cell), ∀ g ∈ Cell.frontiers cs, g ≤ Cell.vals cs
  | [], g, h => by simp [Cell.frontiers] at h
  | c :: cs, g, h => by
      simp only [Cell.frontiers, List.mem_append] at h
      simp only [Cell.vals]
      rcases h with h | h
      · exact le_max_of_le_left (Cell.frontier_le c g h)
      · exact le_max_of_le_right (Cell.frontiers_le cs g h)
end

/-- The delivered order: a recovered root whose published G covers its children error (the
largest over its inputs) is at least every tile of its delivered frontier. -/
theorem root_covers_frontier (kids : List Cell) (G : ℝ) (covers : Cell.vals kids ≤ G) :
    ∀ g ∈ Cell.frontiers kids, g ≤ G :=
  fun g h => (Cell.frontiers_le kids g h).trans covers
