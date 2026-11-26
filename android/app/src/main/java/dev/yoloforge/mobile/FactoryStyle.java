package dev.yoloforge.mobile;

import android.content.Context;
import android.content.res.ColorStateList;
import android.graphics.Typeface;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.InsetDrawable;
import android.graphics.drawable.RippleDrawable;
import android.text.SpannableString;
import android.text.Spanned;
import android.text.style.StyleSpan;
import android.view.Gravity;
import android.widget.Button;
import android.widget.EditText;
import android.widget.TextView;

/** Android counterparts of the desktop palette and component treatments. */
final class FactoryStyle {
    static final int PANEL=0xff16191f, SHELL=0xff13161c, INPUT=0xff12151b, CELL=0xff1b1f27;
    static final int CONTROL=0xff2a303b, SELECTED=0xff1e3045, HOVER=0xff364151, LINE=0xff2c323e;
    static final int TEXT=0xffe7eaf0, MUTED=0xffadb4c0, HINT=0xff747c8a;
    static final int BLUE=0xff368be7, PRIMARY=0xff2d70b4, GREEN=0xff5dcb89, RED=0xfff8746b, AMBER=0xfff4be54;

    private FactoryStyle() {}

    static int dp(Context context,int n) { return Math.round(n*context.getResources().getDisplayMetrics().density); }

    static Drawable fill(Context context,int color,int radius) {
        GradientDrawable shape=new GradientDrawable();shape.setColor(color);shape.setCornerRadius(dp(context,radius));
        return shape;
    }

    static Drawable interactive(Context context,int color,int radius) {
        int[][] states={new int[]{android.R.attr.state_pressed},new int[]{android.R.attr.state_focused},
            new int[]{android.R.attr.state_hovered},new int[]{}};
        return new RippleDrawable(new ColorStateList(states,new int[]{0xa0364151,0x80364151,0x80364151,0x50364151}),
            fill(context,color,radius),fill(context,0xffffffff,radius));
    }

    static TextView label(Context context,CharSequence text,int size,int color) {
        TextView view=new TextView(context);view.setText(text);view.setTextSize(size);view.setTextColor(color);
        return view;
    }

    static Button button(Context context,String text,boolean primary) {
        Button button=new Button(context);
        button.setText(text);button.setAllCaps(false);button.setTextSize(14);button.setTextColor(TEXT);
        button.setTypeface(Typeface.DEFAULT);button.setLetterSpacing(0);
        button.setGravity(Gravity.CENTER);button.setMinHeight(0);button.setMinimumHeight(dp(context,44));
        button.setMinWidth(0);button.setMinimumWidth(0);
        // Keep the touch area taller than the visible button.
        int verticalInset=dp(context,6);
        button.setBackground(new InsetDrawable(interactive(context,primary?PRIMARY:CONTROL,4),0,verticalInset,0,verticalInset));
        int verticalPadding=verticalInset+dp(context,6);
        button.setPaddingRelative(dp(context,14),verticalPadding,dp(context,14),verticalPadding);
        return button;
    }

    static void input(EditText field,String hint) {
        Context context=field.getContext();
        SpannableString placeholder=new SpannableString(hint);
        placeholder.setSpan(new StyleSpan(Typeface.ITALIC),0,hint.length(),Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
        field.setHint(placeholder);field.setHintTextColor(HINT);field.setTextColor(TEXT);field.setTextSize(15);
        android.graphics.drawable.StateListDrawable background=new android.graphics.drawable.StateListDrawable();
        background.addState(new int[]{android.R.attr.state_focused},fieldShape(context,BLUE));
        background.addState(new int[]{},fieldShape(context,LINE));
        field.setBackground(background);
        field.setPaddingRelative(dp(context,13),dp(context,11),dp(context,13),dp(context,11));
        field.setMinHeight(dp(context,44));
    }

    private static Drawable fieldShape(Context context,int stroke) {
        GradientDrawable shape=new GradientDrawable();
        shape.setColor(INPUT);shape.setCornerRadius(dp(context,4));shape.setStroke(dp(context,1),stroke);
        return shape;
    }

    static TextView pill(Context context,String text,int color) {
        TextView view=label(context,text,12,color);
        int tinted=0xff000000 | (((color>>16&255)*28+27*72)/100<<16)
            | (((color>>8&255)*28+31*72)/100<<8) | ((color&255)*28+39*72)/100;
        view.setBackground(fill(context,tinted,6));
        view.setPaddingRelative(dp(context,9),dp(context,4),dp(context,9),dp(context,4));
        view.setMaxLines(1);
        view.setEllipsize(android.text.TextUtils.TruncateAt.END);
        return view;
    }
}
