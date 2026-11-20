import Mathlib.Analysis.Normed.Module.Convex
import Mathlib.Topology.MetricSpace.HausdorffDistance
import Mathlib.Tactic

/- A surface is a set in a normed space. Directed bounds construct a point on
   the other surface, so empty targets cannot silently pass. All distances and
   scores in the theorems are real. The checker uses dyadics as a subset. -/
namespace HonestG
open Set Metric
variable {X : Type*} [PseudoMetricSpace X]

def Directed (s t : Set X) (r : ℝ) : Prop := ∀ x ∈ s, ∃ y ∈ t, dist x y ≤ r
def Both (s t : Set X) (r : ℝ) : Prop := Directed s t r ∧ Directed t s r

theorem Directed.raise {s t : Set X} {r R : ℝ} (h : Directed s t r) (le : r ≤ R) :
    Directed s t R := by
  intro x hx
  obtain ⟨y, hy, hd⟩ := h x hx
  exact ⟨y, hy, hd.trans le⟩

theorem Directed.chain {a b c : Set X} {r l : ℝ} (h : Directed a b r) (k : Directed b c l) :
    Directed a c (r+l) := by
  intro x hx
  obtain ⟨y, hy, hxy⟩ := h x hx
  obtain ⟨z, hz, hyz⟩ := k y hy
  exact ⟨z,hz,(dist_triangle x y z).trans (add_le_add hxy hyz)⟩

theorem Both.chain {a b c : Set X} {r l : ℝ} (h : Both a b r) (k : Both b c l) :
    Both a c (r+l) :=
  ⟨h.1.chain k.1, by simpa [add_comm] using k.2.chain h.2⟩

theorem Both.raise {a b : Set X} {r R : ℝ} (h : Both a b r) (le : r ≤ R) :
    Both a b R := ⟨h.1.raise le, h.2.raise le⟩

theorem Both.hausdorff {a b : Set X} {r : ℝ} (h : Both a b r) (hr : 0 ≤ r) :
    hausdorffDist a b ≤ r := hausdorffDist_le_of_mem_dist hr h.1 h.2

theorem tighten {a b : Set X} {direct composed floor : ℝ}
    (h : Both a b direct) (k : Both a b composed) :
    Both a b (max floor (min direct composed)) := by
  have hmin : Both a b (min direct composed) := by
    rcases le_total direct composed with hdc | hcd
    · simpa [min_eq_left hdc] using h
    · simpa [min_eq_right hcd] using k
  exact hmin.raise (le_max_right ..)

section Faces
variable {V : Type*} [NormedAddCommGroup V] [NormedSpace ℝ V]

def mix (u v w : ℝ) (a b c : V) : V := u • a + v • b + w • c
def face (a b c : V) : Set V :=
  {x | ∃ u v w : ℝ, 0 ≤ u ∧ 0 ≤ v ∧ 0 ≤ w ∧ u+v+w=1 ∧ x=mix u v w a b c}
noncomputable def mid (a b : V) : V := (1/2 : ℝ) • (a+b)

/-- The three proposed target witnesses may be different points, provided
    they lie on one target triangle. Repeated corners and segments are allowed. -/
theorem subface {a b c A B C : V}
    (ha : a ∈ face A B C) (hb : b ∈ face A B C) (hc : c ∈ face A B C) :
    face a b c ⊆ face A B C := by
  rintro x ⟨u,v,w,hu,hv,hw,hs,rfl⟩
  obtain ⟨i,j,k,hi,hj,hk,h1,rfl⟩ := ha
  obtain ⟨l,m,n,hl,hm,hn,h2,rfl⟩ := hb
  obtain ⟨p,q,t,hp,hq,ht,h3,rfl⟩ := hc
  refine ⟨u*i+v*l+w*p,u*j+v*m+w*q,u*k+v*n+w*t,by positivity,by positivity,by positivity,?_,?_⟩
  · linear_combination u*h1 + v*h2 + w*h3 + hs
  · unfold mix
    module

/-- Convex interpolation transports the *entire* domain face into the offset
    of a common target face. No nearest triangle requirement. -/
theorem faceBound {a b c z₀ z₁ z₂ A B C : V} {T : Set V} {r : ℝ}
    (h₀ : z₀ ∈ face A B C) (h₁ : z₁ ∈ face A B C) (h₂ : z₂ ∈ face A B C)
    (ht : face A B C ⊆ T) (d₀ : dist a z₀ ≤ r) (d₁ : dist b z₁ ≤ r) (d₂ : dist c z₂ ≤ r) :
    Directed (face a b c) T r := by
  rintro x ⟨u,v,w,hu,hv,hw,hs,rfl⟩
  refine ⟨mix u v w z₀ z₁ z₂, ht (subface h₀ h₁ h₂ ⟨u,v,w,hu,hv,hw,hs,rfl⟩), ?_⟩
  have he : mix u v w a b c - mix u v w z₀ z₁ z₂ =
      u • (a-z₀) + v • (b-z₁) + w • (c-z₂) := by unfold mix; module
  rw [dist_eq_norm, he]
  calc
    _ ≤ ‖u • (a-z₀) + v • (b-z₁)‖ + ‖w • (c-z₂)‖ := norm_add_le ..
    _ ≤ (‖u • (a-z₀)‖ + ‖v • (b-z₁)‖) + ‖w • (c-z₂)‖ :=
      add_le_add_left (norm_add_le ..) _
    _ = u * dist a z₀ + v * dist b z₁ + w * dist c z₂ := by
      simp [dist_eq_norm, norm_smul, abs_of_nonneg hu, abs_of_nonneg hv, abs_of_nonneg hw]
    _ ≤ u*r + v*r + w*r := by gcongr
    _ = r := by linear_combination r*hs


/-- Matching leaf triangle soup: a per-corner displacement bound is sufficient
    in both directions when all corresponding faces are retained. -/
theorem matchedFace {a b c A B C : V} {r : ℝ}
    (ha : dist a A ≤ r) (hb : dist b B ≤ r) (hc : dist c C ≤ r) :
    Both (face a b c) (face A B C) r := by
  have corners (p q t : V) : p ∈ face p q t ∧ q ∈ face p q t ∧ t ∈ face p q t := by
    exact ⟨⟨1,0,0,by norm_num,by norm_num,by norm_num,by norm_num,by simp [mix]⟩,
           ⟨0,1,0,by norm_num,by norm_num,by norm_num,by norm_num,by simp [mix]⟩,
           ⟨0,0,1,by norm_num,by norm_num,by norm_num,by norm_num,by simp [mix]⟩⟩
  obtain ⟨hA,hB,hC⟩ := corners A B C
  obtain ⟨h₀,h₁,h₂⟩ := corners a b c
  exact ⟨faceBound hA hB hC Subset.rfl ha hb hc, faceBound h₀ h₁ h₂ Subset.rfl
      (by rwa [dist_comm]) (by rwa [dist_comm]) (by rwa [dist_comm])⟩

/-- Exactly the child order used by the binary certificate. -/
theorem fourCover (a b c : V) :
    face a b c ⊆ face a (mid a b) (mid c a) ∪ face (mid a b) b (mid b c) ∪
                 face (mid c a) (mid b c) c ∪ face (mid a b) (mid b c) (mid c a) := by
  rintro x ⟨u,v,w,hu,hv,hw,hs,rfl⟩
  by_cases h : v+w ≤ u
  · left; left; left
    exact ⟨u-v-w,2*v,2*w,by linarith,by positivity,by positivity,by linarith,by unfold mid mix; module⟩
  by_cases h' : u+w ≤ v
  · left; left; right
    exact ⟨2*u,v-u-w,2*w,by positivity,by linarith,by positivity,by linarith,by unfold mid mix; module⟩
  by_cases h'' : u+v ≤ w
  · left; right
    exact ⟨2*u,2*v,w-u-v,by positivity,by positivity,by linarith,by linarith,by unfold mid mix; module⟩
  right
  exact ⟨u+v-w,v+w-u,w+u-v,by linarith,by linarith,by linarith,by linarith,by unfold mid mix; module⟩

/-- A verifier can build this proof tree from valid barycentric and residual checks. -/
inductive Panel (T : Set V) (r : ℝ) : V → V → V → Prop where
  | witness {a b c z₀ z₁ z₂ A B C}
      (h₀ : z₀ ∈ face A B C) (h₁ : z₁ ∈ face A B C) (h₂ : z₂ ∈ face A B C)
      (ht : face A B C ⊆ T) (d₀ : dist a z₀ ≤ r) (d₁ : dist b z₁ ≤ r) (d₂ : dist c z₂ ≤ r) :
      Panel T r a b c
  | split {a b c}
      (one : Panel T r a (mid a b) (mid c a)) (two : Panel T r (mid a b) b (mid b c))
      (three : Panel T r (mid c a) (mid b c) c)
      (four : Panel T r (mid a b) (mid b c) (mid c a)) : Panel T r a b c

theorem Panel.sound {T : Set V} {r : ℝ} {a b c : V} (cert : Panel T r a b c) :
    Directed (face a b c) T r := by
  induction cert with
  | witness h₀ h₁ h₂ ht d₀ d₁ d₂ => exact faceBound h₀ h₁ h₂ ht d₀ d₁ d₂
  | @split a b c _ _ _ _ one two three four =>
    intro x hx
    rcases fourCover a b c hx with ((h|h)|h)|h
    · exact one x h
    · exact two x h
    · exact three x h
    · exact four x h

theorem meshDirected {I : Type*} {a b c : I → V} {T : Set V} {r : ℝ}
    (cert : ∀ i, Panel T r (a i) (b i) (c i)) :
    Directed (⋃ i, face (a i) (b i) (c i)) T r := by
  intro x hx
  obtain ⟨i,hi⟩ := mem_iUnion.mp hx
  exact (cert i).sound x hi
end Faces

def frontier {N : Type*} (kids : Finset N) (surface : N → Set X) : Set X :=
  {x | ∃ k ∈ kids, x ∈ surface k}

theorem directedUnion {N : Type*} {kids : Finset N} (ne : kids.Nonempty)
    {A B : N → Set X} {g : N → ℝ} (h : ∀ k ∈ kids, Directed (A k) (B k) (g k)) :
    Directed (frontier kids A) (frontier kids B) (kids.sup' ne g) := by
  rintro x ⟨k,hk,hx⟩
  obtain ⟨y,hy,hxy⟩ := h k hk x hx
  exact ⟨y,⟨k,hk,hy⟩,hxy.trans (Finset.le_sup' g hk)⟩

/-- Complete children (or recovery frontier) need only cover the same source
    set. There is no requirement that proxy faces be assigned back to children. -/
theorem parentStep {N : Type*} {kids : Finset N} (ne : kids.Nonempty)
    {S P : N → Set X} {g : N → ℝ} {parent : Set X} {localCost : ℝ}
    (children : ∀ k ∈ kids, Both (S k) (P k) (g k))
    (compare : Both (frontier kids P) parent localCost) :
    Both (frontier kids S) parent (kids.sup' ne g + localCost) :=
  Both.chain ⟨directedUnion ne (fun k hk => (children k hk).1),
              directedUnion ne (fun k hk => (children k hk).2)⟩ compare

/-- Evidence read by a bottom-up evaluator. A direct anchor can tighten an
    internal tile; a pure local policy uses anchors only at leaves. -/
def ValidStep {N : Type*} (height : N → ℕ) (S P : N → Set X) (g : N → ℝ) (n : N) : Prop :=
  Both (S n) (P n) (g n) ∨
    ∃ (kids : Finset N) (ne : kids.Nonempty) (localCost : ℝ),
      (∀ k ∈ kids, height k < height n) ∧ S n = frontier kids S ∧
      Both (frontier kids P) (P n) localCost ∧ kids.sup' ne g + localCost ≤ g n

/-- Induction on remaining height; children can skip levels and have unequal heights. -/
theorem depthSound {N : Type*} (height : N → ℕ) (S P : N → Set X) (g : N → ℝ)
    (rule : ∀ n, ValidStep height S P g n)
    (n : N) : Both (S n) (P n) (g n) := by
  induction h : height n using Nat.strong_induction_on generalizing n with
  | ind d ih =>
    rcases rule n with anchor | ⟨kids,ne,l,down,cover,comp,room⟩
    · exact anchor
    · rw [cover]
      exact (parentStep ne (fun k hk => ih (height k) (h ▸ down k hk) k rfl) comp).raise room

theorem publishedBoundsHausdorff {N : Type*} (height : N → ℕ) (S P : N → Set X) (g : N → ℝ)
    (rule : ∀ n, ValidStep height S P g n) (published : N → ℝ)
    (nonneg : ∀ n, 0 ≤ g n) (floorOK : ∀ n, g n ≤ published n) (n : N) :
    hausdorffDist (S n) (P n) ≤ published n :=
  ((depthSound height S P g rule n).hausdorff (nonneg n)).trans (floorOK n)

/-- Same cover, tighter accounting: a forward source point has a child of
    origin. A reverse panel certified against a single child face can charge
    the radius of that witness's child, even if other children are far worse. -/
def Pair (s p : Set X) (f b : ℝ) : Prop := Directed s p f ∧ Directed p s b

def LocalCharges {N : Type*} (kids : Finset N) (P : N → Set X) (parent : Set X)
    (f b : N → ℝ) (F B : ℝ) : Prop :=
  (∀ i ∈ kids, ∃ l : ℝ, Directed (P i) parent l ∧ f i + l ≤ F) ∧
  (∀ x ∈ parent, ∃ i ∈ kids, ∃ y ∈ P i, ∃ l : ℝ, dist x y ≤ l ∧ l + b i ≤ B)

theorem chargedParent {N : Type*} {kids : Finset N} {S P : N → Set X} {parent : Set X}
    {f b : N → ℝ} {F B : ℝ} (hc : ∀ i ∈ kids, Pair (S i) (P i) (f i) (b i))
    (localCost : LocalCharges kids P parent f b F B) : Pair (frontier kids S) parent F B := by
  constructor
  · rintro x ⟨i,hi,hx⟩
    obtain ⟨l,h,room⟩ := localCost.1 i hi
    exact (((hc i hi).1.chain h).raise room) x hx
  · intro x hx
    obtain ⟨i,hi,y,hy,l,hd,room⟩ := localCost.2 x hx
    obtain ⟨z,hz,hd'⟩ := (hc i hi).2 y hy
    exact ⟨z,⟨i,hi,hz⟩,(dist_triangle x y z).trans ((add_le_add hd hd').trans room)⟩

def WeightedStep {N : Type*} (height : N → ℕ) (S P : N → Set X) (f b : N → ℝ) (n : N) : Prop :=
  Pair (S n) (P n) (f n) (b n) ∨ ∃ kids : Finset N,
    (∀ i ∈ kids, height i < height n) ∧ S n = frontier kids S ∧
    LocalCharges kids P (P n) f b (f n) (b n)

theorem chargedDepth {N : Type*} (height : N → ℕ) (S P : N → Set X) (f b : N → ℝ)
    (rule : ∀ n, WeightedStep height S P f b n) (n : N) : Pair (S n) (P n) (f n) (b n) := by
  induction h : height n using Nat.strong_induction_on generalizing n with
  | ind d ih =>
    rcases rule n with anchor | ⟨kids,down,cover,localCost⟩
    · exact anchor
    · rw [cover]
      exact chargedParent (fun k hk => ih (height k) (h ▸ down k hk) k rfl) localCost

theorem weightedPublished {N : Type*} (height : N → ℕ) (S P : N → Set X) (f b : N → ℝ)
    (rule : ∀ n, WeightedStep height S P f b n) (n : N) (G : ℝ) (hG : 0 ≤ G)
    (hF : f n ≤ G) (hB : b n ≤ G) : hausdorffDist (S n) (P n) ≤ G := by
  have h := chargedDepth height S P f b rule n
  exact hausdorffDist_le_of_mem_dist hG (h.1.raise hF) (h.2.raise hB)

theorem Directed.takeMin {s t : Set X} {r v : ℝ} (h : Directed s t r) (k : Directed s t v) :
    Directed s t (min r v) := by
  rcases le_total r v with le | le
  · simpa [min_eq_left le] using h
  · simpa [min_eq_right le] using k

#print axioms chargedParent
#print axioms chargedDepth
#print axioms weightedPublished
#print axioms Directed.takeMin
#print axioms matchedFace
#print axioms publishedBoundsHausdorff
#print axioms faceBound
#print axioms Panel.sound
#print axioms meshDirected
#print axioms depthSound
#print axioms tighten
#print axioms Both.hausdorff
end HonestG
