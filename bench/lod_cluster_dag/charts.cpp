// Research planar charts: fixed normal cone, interior-overlap rejection,
// bounded connected growth, shelf packing and triangle raster coordinates.
// Adapted from this repository's v0.3.0 texture.rs and current bake/charts.rs.
#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <numeric>
#include <unordered_map>
#include <vector>
using V=std::array<double,3>; using P=std::array<double,2>; using T=std::array<P,3>;
V sub(V a,V b){return {a[0]-b[0],a[1]-b[1],a[2]-b[2]};}
double dot(V a,V b){return a[0]*b[0]+a[1]*b[1]+a[2]*b[2];}
V cross(V a,V b){return {a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]};}
V unit(V a){double d=sqrt(dot(a,a));return d>0?V{a[0]/d,a[1]/d,a[2]/d}:V{0,1,0};}
bool overlap(const T&a,const T&b){
 for(const auto*t:{&a,&b})for(int k=0;k<3;k++){
  P e={(*t)[(k+1)%3][0]-(*t)[k][0],(*t)[(k+1)%3][1]-(*t)[k][1]};
  P axis={-e[1],e[0]};double al=1e100,ah=-al,bl=al,bh=-al;
  for(int j=0;j<3;j++){double x=a[j][0]*axis[0]+a[j][1]*axis[1],y=b[j][0]*axis[0]+b[j][1]*axis[1];al=std::min(al,x);ah=std::max(ah,x);bl=std::min(bl,y);bh=std::max(bh,y);}
  if(std::min(ah,bh)-std::max(al,bl)<=1e-10*std::max(1.,sqrt(axis[0]*axis[0]+axis[1]*axis[1])))return false;
 }return true;
}
struct Chart{std::vector<int> faces;std::vector<T> points; P lo,hi;int x=0,y=0,w=0,h=0;};
extern "C" int unwrap(const float*pos,const uint32_t*idx,int nf,int size,float*uv,int32_t*face,float*bc){
 std::vector<V> normals(nf);std::vector<double> area(nf);std::vector<std::vector<int>>adj(nf);
 auto vertex=[&](int f,int k){auto i=idx[f*3+k];return V{pos[i*3],pos[i*3+1],pos[i*3+2]};};
 std::unordered_map<uint64_t,std::vector<int>> edges;
 for(int f=0;f<nf;f++){
  V n=cross(sub(vertex(f,1),vertex(f,0)),sub(vertex(f,2),vertex(f,0)));area[f]=sqrt(dot(n,n));normals[f]=unit(n);
  for(int k=0;k<3;k++){uint32_t a=idx[f*3+k],b=idx[f*3+(k+1)%3];if(a>b)std::swap(a,b);edges[(uint64_t(a)<<32)|b].push_back(f);}
 }
 for(auto &e:edges)for(int a:e.second)for(int b:e.second)if(a!=b)adj[a].push_back(b);
 for(auto&a:adj){std::sort(a.begin(),a.end());a.erase(std::unique(a.begin(),a.end()),a.end());}
 std::vector<int> order(nf),assigned(nf,-1);std::iota(order.begin(),order.end(),0);
 std::stable_sort(order.begin(),order.end(),[&](int a,int b){return area[a]>area[b];});std::vector<Chart>charts;
 for(int seed:order){if(assigned[seed]>=0)continue;
  V normal=normals[seed],axis=std::abs(normal[0])<.8?V{1,0,0}:V{0,1,0};V x=unit(cross(axis,normal)),y=cross(normal,x),origin=vertex(seed,0);
  Chart c;c.lo={1e100,1e100};c.hi={-1e100,-1e100};std::vector<int>queue{seed};
  for(size_t q=0;q<queue.size();q++){
   int f=queue[q];if(assigned[f]>=0||dot(normal,normals[f])<.7||c.faces.size()>=128)continue;
   T p;P lo=c.lo,hi=c.hi;for(int k=0;k<3;k++){V v=sub(vertex(f,k),origin);p[k]={dot(v,x),dot(v,y)};for(int a=0;a<2;a++){lo[a]=std::min(lo[a],p[k][a]);hi[a]=std::max(hi[a],p[k][a]);}}
   bool hit=false;for(auto&t:c.points)if(overlap(p,t)){hit=true;break;}if(hit)continue;
   assigned[f]=charts.size();c.faces.push_back(f);c.points.push_back(p);c.lo=lo;c.hi=hi;
   for(int nb:adj[f])if(assigned[nb]<0)queue.push_back(nb);
  }
  // Degenerate faces still need a chart and valid identity; they contribute no raster area.
  if(c.faces.empty()){c.faces={seed};c.points={T{P{0,0},P{0,0},P{0,0}}};c.lo={0,0};c.hi={0,0};assigned[seed]=charts.size();}
  charts.push_back(std::move(c));
 }
 std::vector<int>co(charts.size());std::iota(co.begin(),co.end(),0);
 auto pack=[&](double scale){
  for(auto&c:charts){c.w=int(ceil((c.hi[0]-c.lo[0])*scale))+5;c.h=int(ceil((c.hi[1]-c.lo[1])*scale))+5;}
  std::stable_sort(co.begin(),co.end(),[&](int a,int b){return charts[a].h>charts[b].h;});int xx=0,yy=0,row=0;
  for(int id:co){auto&c=charts[id];if(c.w>size||c.h>size)return false;if(xx+c.w>size){xx=0;yy+=row;row=0;}if(yy+c.h>size)return false;c.x=xx;c.y=yy;xx+=c.w;row=std::max(row,c.h);}return true;
 };
 if(!pack(0))return -1;double low=0,high=1;while(pack(high)&&high<1e6)high*=2;
 for(int it=0;it<24;it++){double mid=(low+high)/2;if(pack(mid))low=mid;else high=mid;}pack(low);
 std::fill(face,face+size*size,-1);std::fill(bc,bc+size*size*3,0.f);
 for(auto&c:charts)for(size_t a=0;a<c.faces.size();a++){
  int f=c.faces[a];T p;for(int k=0;k<3;k++)for(int z=0;z<2;z++){p[k][z]=(c.points[a][k][z]-c.lo[z])*low+(z?c.y:c.x)+2.5;uv[f*6+k*2+z]=p[k][z]/size;}
  double den=(p[1][1]-p[2][1])*(p[0][0]-p[2][0])+(p[2][0]-p[1][0])*(p[0][1]-p[2][1]);if(std::abs(den)<1e-20)continue;
  int x0=std::max(0,int(floor(std::min({p[0][0],p[1][0],p[2][0]})))),x1=std::min(size-1,int(ceil(std::max({p[0][0],p[1][0],p[2][0]}))));
  int y0=std::max(0,int(floor(std::min({p[0][1],p[1][1],p[2][1]})))),y1=std::min(size-1,int(ceil(std::max({p[0][1],p[1][1],p[2][1]}))));
  for(int yy=y0;yy<=y1;yy++)for(int xx=x0;xx<=x1;xx++){
   double u=((p[1][1]-p[2][1])*(xx+.5-p[2][0])+(p[2][0]-p[1][0])*(yy+.5-p[2][1]))/den;
   double v=((p[2][1]-p[0][1])*(xx+.5-p[2][0])+(p[0][0]-p[2][0])*(yy+.5-p[2][1]))/den,w=1-u-v;
   if(std::min({u,v,w})>=-1e-7){int i=yy*size+xx;face[i]=f;bc[i*3]=u;bc[i*3+1]=v;bc[i*3+2]=w;}
  }
 }
 return charts.size();
}
