#include <math.h>

/* The second argument is widened to long double by the C call, as jq does.
   That width is 80 bits on x86_64 and the same as double on arm64. */
double tumulus_nexttoward(double x, double y) {
    return nexttoward(x, (long double)y);
}
