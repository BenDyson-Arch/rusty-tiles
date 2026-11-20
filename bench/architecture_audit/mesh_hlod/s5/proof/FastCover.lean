/-
Copyright (c) 2023 Sébastien Gouëzel. The proof of `punctured_convex` adapts
Mathlib's `Set.Countable.isPathConnected_compl_of_one_lt_rank` to an open convex
domain. That portion is used under the Apache License, Version 2.0, reproduced
in `vendor/basis_universal/LICENSES/Apache-2.0.txt`.
-/
import HonestG
import Mathlib.Topology.Connected.Clopen
import Mathlib.Analysis.Normed.Module.Convex
import Mathlib.Analysis.Normed.Module.Connected
import Mathlib.Analysis.InnerProductSpace.PiL2
import Mathlib.Tactic

namespace FastCover
open Set Filter Topology
open scoped Convex

/-- Strict side inequalities that hold at a point continue to hold in an open neighborhood. -/
theorem strict_nhds {P I : Type*} [TopologicalSpace P] [Fintype I]
    (H : I → P → ℝ) (cont : ∀ i, Continuous (H i)) {p : P} (strict : ∀ i, 0 < H i p) :
    {q | ∀ i, 0 < H i q} ∈ 𝓝 p := by
  have eq : {q | ∀ i, 0 < H i q} = ⋂ i, {q | 0 < H i q} := by ext; simp
  rw [eq]
  exact (isOpen_iInter_of_finite fun i => isOpen_lt continuous_const (cont i)).mem_nhds
    (by simp [strict])

/-- Across a glued non-folding projected edge, the two incident triangles cover an
open neighborhood of every point strictly inside the other four sides. The common
edge functional can have either sign at a point in that neighborhood. -/
theorem seam {P : Type*} [TopologicalSpace P] (edge : P → ℝ) (a b : Fin 2 → P → ℝ)
    (ca : ∀ i, Continuous (a i)) (cb : ∀ i, Continuous (b i)) {p : P}
    (ha : ∀ i, 0 < a i p) (hb : ∀ i, 0 < b i p) :
    ({q | 0 ≤ edge q ∧ ∀ i, 0 ≤ a i q} ∪ {q | edge q ≤ 0 ∧ ∀ i, 0 ≤ b i q}) ∈ 𝓝 p := by
  filter_upwards [strict_nhds a ca ha, strict_nhds b cb hb] with q qa qb
  rcases le_total 0 (edge q) with h | h
  · exact Or.inl ⟨h, fun i => (qa i).le⟩
  · exact Or.inr ⟨h, fun i => (qb i).le⟩

/-- The projection of a finite union of triangles is closed. On the open interior
of the query triangle minus the finitely many projected mesh vertices, each interior
point of a visited face has an open neighborhood, and each edge point has the two-face
neighborhood above. Connectedness and one overlap then cover the entire closure. -/
theorem closed_cover {P : Type*} [TopologicalSpace P] {D U : Set P}
    (connected : IsPreconnected D) (closed : IsClosed U)
    (inside : ∀ p ∈ D ∩ U, U ∈ 𝓝 p) (hit : (D ∩ U).Nonempty) :
    closure D ⊆ U := by
  let : PreconnectedSpace D := isPreconnected_iff_preconnectedSpace.mp connected
  have op : IsOpen ((Subtype.val : D → P) ⁻¹' U) := by
    apply isOpen_iff_mem_nhds.mpr
    intro x hx
    exact continuous_subtype_val.continuousAt.preimage_mem_nhds (inside x ⟨x.property, hx⟩)
  have cl : IsClosed ((Subtype.val : D → P) ⁻¹' U) := closed.preimage continuous_subtype_val
  obtain ⟨p,hp,hu⟩ := hit
  have eq := IsClopen.eq_univ ⟨cl,op⟩ (show ((Subtype.val : D → P) ⁻¹' U).Nonempty from
    ⟨⟨p,hp⟩,hu⟩)
  apply closure_minimal _ closed
  intro z hz
  have zz : (⟨z,hz⟩ : D) ∈ ((Subtype.val : D → P) ⁻¹' U) := by rw [eq]; trivial
  exact zz

/-- A projected footprint with an actual height-matched witness on each visited target
face is a complete directed-distance certificate on the original three-dimensional set. -/
theorem projected_direction {X P : Type*} [PseudoMetricSpace X] (proj : X → P)
    {source patch target : Set X} {r : ℝ} (covered : proj '' source ⊆ proj '' patch)
    (owned : patch ⊆ target)
    (separation : ∀ x ∈ source, ∀ y ∈ patch, proj x = proj y → dist x y ≤ r) :
    HonestG.Directed source target r := by
  intro x hx
  obtain ⟨y,hy,hpos⟩ := covered ⟨x,hx,rfl⟩
  exact ⟨y,owned hy,separation x hx y hy hpos.symm⟩

/-- Subtract any common affine function of the two retained coordinates. At a common
projection the affine terms cancel. Extreme residuals suffice for a vertical witness. -/
theorem height_bound (a b h lo hi low high : ℝ) (ha : lo ≤ a-h ∧ a-h ≤ hi)
    (hb : low ≤ b-h ∧ b-h ≤ high) : |a-b| ≤ max (hi-low) (high-lo) := by
  rw [abs_le]
  constructor
  · have := le_max_right (hi-low) (high-lo)
    linarith [ha.1, hb.2]
  · have := le_max_left (hi-low) (high-lo)
    linarith [ha.2, hb.1]

/-- A true lower observation can supply a relative target. An independent publication
floor can absorb a cover below that floor. Honesty only needs the complete upper cover. -/
theorem target_gap (truth lower upper floor eps : ℝ) (he : 0 ≤ eps) (hf : 0 ≤ floor)
    (hl : lower ≤ truth) (hu : truth ≤ upper)
    (stopped : upper ≤ max floor ((1+eps)*lower)) :
    truth ≤ max floor upper ∧ max floor upper ≤ (1+eps)*max floor truth := by
  constructor
  · exact hu.trans (le_max_right ..)
  · have hF : floor ≤ (1+eps)*max floor truth := by
      calc floor ≤ 1*max floor truth := by simp
           _ ≤ _ := mul_le_mul_of_nonneg_right (by linarith) (hf.trans (le_max_left ..))
    exact max_le hF (stopped.trans (max_le hF
      (mul_le_mul_of_nonneg_left (hl.trans (le_max_right ..)) (by linarith))))

#print axioms projected_direction
#print axioms seam
#print axioms closed_cover
#print axioms height_bound
#print axioms target_gap

/-- The open convex interior of a planar face or convex hull stays path-connected
after removing a finite set of projected vertices. The two-segment proof adapts
Mathlib's countable-complement lemma in Analysis.Normed.Module.Connected
(Apache-2.0), choosing its middle point inside the open interior. -/
theorem punctured_convex {E : Type*} [NormedAddCommGroup E] [NormedSpace ℝ E]
    (h : 1 < Module.rank ℝ E) {U s : Set E} (op : IsOpen U) (cv : Convex ℝ U)
    (hs : s.Countable) (hit : (U ∩ sᶜ).Nonempty) : IsPathConnected (U ∩ sᶜ) := by
  obtain ⟨a,ha⟩ := hit
  refine ⟨a,ha,?_⟩
  intro b hb
  rcases eq_or_ne a b with rfl | hab
  · exact JoinedIn.refl ha
  let c := (2 : ℝ)⁻¹ • (a + b)
  let x := (2 : ℝ)⁻¹ • (b - a)
  have Ia : c - x = a := by
    simp only [c, x]
    module
  have Ib : c + x = b := by
    simp only [c, x]
    module
  have x_ne_zero : x ≠ 0 := by simpa [x] using sub_ne_zero.2 hab.symm
  obtain ⟨y, hy⟩ : ∃ y, LinearIndependent ℝ ![x, y] :=
    exists_linearIndependent_pair_of_one_lt_rank h x_ne_zero
  have A : Set.Countable {t : ℝ | ([c + x -[ℝ] c + t • y] ∩ s).Nonempty} := by
    apply countable_ofPred_nonempty_of_disjoint _ (fun t ↦ inter_subset_right) hs
    intro t t' htt'
    apply disjoint_iff_inter_eq_empty.2
    have N : {c + x} ∩ s = ∅ := by
      simpa only [singleton_inter_eq_empty, mem_compl_iff, Ib] using hb.2
    rw [inter_assoc, inter_comm s, inter_assoc, inter_self, ← inter_assoc, ← subset_empty_iff, ← N]
    apply inter_subset_inter_left
    apply Eq.subset
    apply segment_inter_eq_endpoint_of_linearIndependent_of_ne hy htt'.symm
  have B : Set.Countable {t : ℝ | ([c - x -[ℝ] c + t • y] ∩ s).Nonempty} := by
    apply countable_ofPred_nonempty_of_disjoint _ (fun t ↦ inter_subset_right) hs
    intro t t' htt'
    apply disjoint_iff_inter_eq_empty.2
    have N : {c - x} ∩ s = ∅ := by
      simpa only [singleton_inter_eq_empty, mem_compl_iff, Ia] using ha.2
    rw [inter_assoc, inter_comm s, inter_assoc, inter_self, ← inter_assoc, ← subset_empty_iff, ← N]
    apply inter_subset_inter_left
    rw [sub_eq_add_neg _ x]
    apply Eq.subset
    apply segment_inter_eq_endpoint_of_linearIndependent_of_ne _ htt'.symm
    convert hy.units_smul ![-1, 1]
    simp [← List.ofFn_inj]
  have cm : c ∈ U := by
    have := cv.midpoint_mem ha.1 hb.1
    simpa [c,midpoint_eq_smul_add] using this
  have around : {t : ℝ | c + t • y ∈ U} ∈ 𝓝 (0:ℝ) := by
    apply ((continuous_const.add (continuous_id.smul continuous_const)).isOpen_preimage _ op).mem_nhds
    change c + (0 : ℝ) • y ∈ U
    simpa only [zero_smul, add_zero] using cm
  obtain ⟨t,ht,hin⟩ := ((A.union B).dense_compl ℝ).inter_nhds_nonempty around
  let z := c+t • y
  simp only [compl_union, mem_inter_iff, mem_compl_iff, mem_ofPred_eq, not_nonempty_iff_eq_empty] at ht
  have paths (v : E) (hv : v ∈ U) (avoid : [v -[ℝ] z] ∩ s = ∅) :
      JoinedIn (U ∩ sᶜ) v z := by
    apply JoinedIn.of_segment_subset
    apply subset_inter (cv.segment_subset hv hin)
    rwa [subset_compl_iff_disjoint_right,disjoint_iff_inter_eq_empty]
  exact (paths a ha.1 (by simpa only [Ia,z] using ht.2)).trans
    (paths b hb.1 (by simpa only [Ib,z] using ht.1)).symm

/-- The closure restores every omitted vertex and the closed boundary. -/
theorem unpuncture {E : Type*} [NormedAddCommGroup E] [NormedSpace ℝ E] [Nontrivial E]
    {U s : Set E} (op : IsOpen U) (hs : s.Countable) :
    closure (U ∩ sᶜ) = closure U := by
  apply le_antisymm (closure_mono inter_subset_left)
  apply closure_minimal _ isClosed_closure
  have h := op.inter_closure (t:=sᶜ)
  simpa [(hs.dense_compl ℝ).closure_eq] using h

#print axioms punctured_convex
#print axioms unpuncture

/-- On a common projection the displacement along the omitted coordinate is
exactly the Euclidean distance of these two three-dimensional witnesses. -/
theorem axial_distance (p q : EuclideanSpace ℝ (Fin 3)) (k : Fin 3)
    (same : ∀ j, j ≠ k → p j = q j) : dist p q = |p k - q k| := by
  rw [EuclideanSpace.dist_eq, Finset.sum_eq_single k]
  · rw [Real.sqrt_sq dist_nonneg, Real.dist_eq]
  · intro j _ hj
    rw [same j hj, dist_self]
    norm_num
  · simp

/-- The same interval bounds every convex blend of the three vertex residuals. -/
theorem corner_interval {a b c lo hi u v w : ℝ}
    (ha : lo ≤ a ∧ a ≤ hi) (hb : lo ≤ b ∧ b ≤ hi) (hc : lo ≤ c ∧ c ≤ hi)
    (hu : 0 ≤ u) (hv : 0 ≤ v) (hw : 0 ≤ w) (sum : u + v + w = 1) :
    lo ≤ u*a + v*b + w*c ∧ u*a + v*b + w*c ≤ hi := by
  have factor (x : ℝ) : u*x + v*x + w*x = x := by
    rw [← add_mul, ← add_mul, sum, one_mul]
  constructor
  · calc lo = u*lo + v*lo + w*lo := (factor _).symm
         _ ≤ _ := add_le_add (add_le_add (mul_le_mul_of_nonneg_left ha.1 hu)
           (mul_le_mul_of_nonneg_left hb.1 hv)) (mul_le_mul_of_nonneg_left hc.1 hw)
  · calc _ ≤ u*hi + v*hi + w*hi := add_le_add (add_le_add (mul_le_mul_of_nonneg_left ha.2 hu)
           (mul_le_mul_of_nonneg_left hb.2 hv)) (mul_le_mul_of_nonneg_left hc.2 hw)
         _ = hi := factor _

/-- Separate affine functions over a common bounding box. Their difference
is bracketed there. This also covers the equal-slope corner enclosure. -/
theorem different_slopes {a b A B al ah bl bh dl dh : ℝ}
    (ha : al ≤ a-A ∧ a-A ≤ ah) (hb : bl ≤ b-B ∧ b-B ≤ bh)
    (delta : dl ≤ A-B ∧ A-B ≤ dh) : |a-b| ≤ max (ah-bl+dh) (bh-al-dl) := by
  rw [abs_le]
  constructor
  · linarith [le_max_right (ah-bl+dh) (bh-al-dl), ha.1, hb.2, delta.1]
  · linarith [le_max_left (ah-bl+dh) (bh-al-dl), ha.2, hb.1, delta.2]

#print axioms axial_distance
#print axioms corner_interval
#print axioms different_slopes

end FastCover
