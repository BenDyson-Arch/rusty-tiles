// Independent double-precision point/triangle BVH for sampled (not certified) error.
// Usage: nearest TRIANGLES.f64 QUERY_POINTS.f64 OUT.f64
// Output records: distance, triangle index, barycentric b0, b1, b2.
#include <algorithm>
#include <array>
#include <cmath>
#include <fstream>
#include <numeric>
#include <vector>
#include <limits>
#include <iostream>
using V=std::array<double,3>;using T=std::array<V,3>;
V add(V a,V b){for(int k=0;k<3;k++)a[k]+=b[k];return a;}V sub(V a,V b){for(int k=0;k<3;k++)a[k]-=b[k];return a;}V mul(V a,double b){for(auto &v:a)v*=b;return a;}double dot(V a,V b){return a[0]*b[0]+a[1]*b[1]+a[2]*b[2];}
V closest(V p,T t){auto a=t[0],b=t[1],c=t[2],ab=sub(b,a),ac=sub(c,a),ap=sub(p,a);double d1=dot(ab,ap),d2=dot(ac,ap);if(d1<=0&&d2<=0)return {1,0,0};auto bp=sub(p,b);double d3=dot(ab,bp),d4=dot(ac,bp);if(d3>=0&&d4<=d3)return {0,1,0};double vc=d1*d4-d3*d2;if(vc<=0&&d1>=0&&d3<=0){double v=d1/(d1-d3);return {1-v,v,0};}auto cp=sub(p,c);double d5=dot(ab,cp),d6=dot(ac,cp);if(d6>=0&&d5<=d6)return {0,0,1};double vb=d5*d2-d1*d6;if(vb<=0&&d2>=0&&d6<=0){double w=d2/(d2-d6);return {1-w,0,w};}double va=d3*d6-d5*d4;if(va<=0&&(d4-d3)>=0&&(d5-d6)>=0){double w=(d4-d3)/((d4-d3)+(d5-d6));return {0,1-w,w};}double den=va+vb+vc;if(den==0)return {1,0,0};return {1-(vb+vc)/den,vb/den,vc/den};}
struct N{V lo,hi;int a,b,left=-1,right=-1;};std::vector<T>tris;std::vector<int>order;std::vector<N>nodes;
int build(int a,int b){N n{{INFINITY,INFINITY,INFINITY},{-INFINITY,-INFINITY,-INFINITY},a,b};for(int i=a;i<b;i++)for(auto p:tris[order[i]])for(int k=0;k<3;k++){n.lo[k]=std::min(n.lo[k],p[k]);n.hi[k]=std::max(n.hi[k],p[k]);}int id=nodes.size();nodes.push_back(n);if(b-a>8){int k=0;for(int c=1;c<3;c++)if(n.hi[c]-n.lo[c]>n.hi[k]-n.lo[k])k=c;int m=(a+b)/2;std::nth_element(order.begin()+a,order.begin()+m,order.begin()+b,[&](int x,int y){return tris[x][0][k]+tris[x][1][k]+tris[x][2][k]<tris[y][0][k]+tris[y][1][k]+tris[y][2][k];});int l=build(a,m),r=build(m,b);nodes[id].left=l;nodes[id].right=r;}return id;}
double bound(V p,int ni){auto n=nodes[ni];double d=0;for(int k=0;k<3;k++){double a=std::max({0.,n.lo[k]-p[k],p[k]-n.hi[k]});d+=a*a;}return d;}
void query(int ni,V p,double &best,int &win,V &bary){auto n=nodes[ni];if(bound(p,ni)>best+1e-20)return;if(n.left<0){for(int i=n.a;i<n.b;i++){int j=order[i];V b=closest(p,tris[j]),q{};for(int c=0;c<3;c++)q=add(q,mul(tris[j][c],b[c]));double d=dot(sub(p,q),sub(p,q));if(d<best){best=d;win=j;bary=b;}}}else{int a=n.left,b=n.right;if(bound(p,a)>bound(p,b))std::swap(a,b);query(a,p,best,win,bary);query(b,p,best,win,bary);}}
template<class A>std::vector<A> read(const char *p){std::ifstream f(p,std::ios::binary|std::ios::ate);if(!f)throw std::runtime_error(p);size_t n=f.tellg();if(n%sizeof(A))throw std::runtime_error("shape");f.seekg(0);std::vector<A>a(n/sizeof(A));f.read((char*)a.data(),n);return a;}
int main(int argc,char**argv){if(argc!=4)return 2;tris=read<T>(argv[1]);if(tris.empty())return 3;auto points=read<V>(argv[2]);order.resize(tris.size());std::iota(order.begin(),order.end(),0);build(0,tris.size());std::ofstream out(argv[3],std::ios::binary);for(auto p:points){double d=INFINITY;int win=-1;V b{};query(0,p,d,win,b);std::array<double,5>r{sqrt(d),double(win),b[0],b[1],b[2]};out.write((char*)r.data(),sizeof(r));}std::cerr<<tris.size()<<" triangles, "<<points.size()<<" samples\n";}
