using System.Text;
using Ironpress;

internal static class Program
{
    private static readonly (string Name, Action<TestContext> Run)[] Tests =
    [
        ("native contract identifies the package and ABI", NativeContractMatchesPackage),
        ("portable options compose into one conversion", PortableOptionsCompose),
        ("font packs cross the managed boundary", FontPacksCrossManagedBoundary),
        ("native failures retain their category", NativeFailuresRetainCategory),
        ("disposed converters reject further work", DisposedConvertersRejectWork),
        ("equivalent converters produce identical bytes", EquivalentConvertersMatch),
        ("repeated ownership cycles remain valid", RepeatedOwnershipCyclesRemainValid),
    ];

    private static int Main(string[] arguments)
    {
        if (arguments.Length != 2)
        {
            Console.Error.WriteLine("Expected the custom-font and font-pack fixture paths.");
            return 2;
        }

        var context = new TestContext(arguments[0], arguments[1]);
        foreach (var test in Tests)
        {
            try
            {
                test.Run(context);
                Console.WriteLine($"PASS: {test.Name}");
            }
            catch (Exception error)
            {
                Console.Error.WriteLine($"FAIL: {test.Name}\n{error}");
                return 1;
            }
        }

        return 0;
    }

    private static void NativeContractMatchesPackage(TestContext _)
    {
        Equal(1u, IronpressInfo.AbiVersion, "Unexpected native ABI generation.");

        var expectedVersion = Environment.GetEnvironmentVariable("IRONPRESS_EXPECTED_VERSION");
        if (expectedVersion is not null)
        {
            Equal(expectedVersion, IronpressInfo.Version, "Native package version differs.");
        }
    }

    private static void PortableOptionsCompose(TestContext context)
    {
        using var converter = new HtmlConverter()
            .SetPageSize(PageSize.Letter)
            .SetCustomPageSize(PageDimensions.FromPoints(320, 480))
            .SetMargins(PageMargins.FromPoints(12, 13, 14, 15))
            .SetCompression(false)
            .SetJpegQuality(82)
            .SetAutomaticImageResize(false)
            .SetImageResolution(144)
            .SetFilterResolution(96)
            .SetMaskResolution(144)
            .SetBackgroundResolution(120)
            .SetOcclusionCulling(true)
            .SetSanitization(true)
            .SetHeader("Contract header")
            .SetHeaderHtml("<strong>Contract HTML header</strong>")
            .SetFooter("Page {page} of {pages}")
            .SetFooterHtml("<em>Contract HTML footer</em>")
            .AddFont("ParitySans", File.ReadAllBytes(context.CustomFontPath))
            .AddFontPack(
                FontPackKind.CjkJapanese,
                File.ReadAllBytes(context.FontPackPath));

        var pdf = converter.ConvertHtml(
            "<h1 style='font-family:ParitySans'>.NET binding</h1><p lang='ja'>第</p>");
        StartsWithPdf(pdf);
        Contains(pdf, "/MediaBox [0 0 320 480]", "Custom page size was not applied.");

        converter.SetAccessibility(true).SetDocumentTitle("Contract").SetDocumentLanguage("en-GB");
        Contains(
            converter.ConvertHtml("<p>Accessible</p>"),
            "/StructTreeRoot",
            "Accessible output has no structure tree.");

        StartsWithPdf(converter.ConvertMarkdown("# Markdown binding"));
    }

    private static void FontPacksCrossManagedBoundary(TestContext context)
    {
        using var converter = new HtmlConverter()
            .AddFontPack(FontPackKind.CjkJapanese, File.ReadAllBytes(context.FontPackPath));

        var pdf = converter.ConvertHtml("<p lang='ja'>第</p>");
        Contains(pdf, "DroidSansFallback", "The supplied fallback font was not embedded.");
    }

    private static void NativeFailuresRetainCategory(TestContext _)
    {
        using var converter = new HtmlConverter();

        var error = Throws<IronpressException>(() => converter.SetPageSize((PageSize)999));
        Equal(IronpressErrorKind.InvalidEnum, error.Kind, "Native error category changed.");

        var fontError = Throws<IronpressException>(() =>
            converter.AddFontPack(FontPackKind.Emoji, "not a font"u8));
        Equal(IronpressErrorKind.Font, fontError.Kind, "Font error category changed.");

        Throws<ArgumentOutOfRangeException>(() => PageDimensions.FromPoints(0, 100));
        Throws<ArgumentOutOfRangeException>(() => converter.SetMargin(float.NaN));
        Throws<ArgumentException>(() => converter.SetHeader("\ud800"));

        var languageError = Throws<IronpressException>(() => converter.SetDocumentLanguage("1234"));
        Equal(IronpressErrorKind.InvalidArgument, languageError.Kind, "Language error category changed.");
    }

    private static void DisposedConvertersRejectWork(TestContext _)
    {
        var converter = new HtmlConverter();
        converter.Dispose();

        Throws<ObjectDisposedException>(() => converter.ConvertHtml("<p>too late</p>"));
    }

    private static void EquivalentConvertersMatch(TestContext _)
    {
        using var first = new HtmlConverter().SetCompression(false).SetMargin(24);
        using var second = new HtmlConverter().SetCompression(false).SetMargin(24);

        var source = "<h1>Deterministic contract</h1>";
        SequenceEqual(first.ConvertHtml(source), second.ConvertHtml(source));
    }

    private static void RepeatedOwnershipCyclesRemainValid(TestContext _)
    {
        for (var iteration = 0; iteration < 25; iteration++)
        {
            using var converter = new HtmlConverter();
            StartsWithPdf(converter.ConvertHtml($"<p>cycle {iteration}</p>"));
        }
    }

    private static void StartsWithPdf(byte[] bytes)
    {
        if (bytes.Length < 4 || Encoding.ASCII.GetString(bytes, 0, 4) != "%PDF")
        {
            throw new InvalidOperationException("Conversion did not return PDF bytes.");
        }
    }

    private static void Contains(byte[] bytes, string expected, string message)
    {
        if (!Encoding.Latin1.GetString(bytes).Contains(expected, StringComparison.Ordinal))
        {
            throw new InvalidOperationException(message);
        }
    }

    private static TException Throws<TException>(Action action)
        where TException : Exception
    {
        try
        {
            action();
        }
        catch (TException error)
        {
            return error;
        }

        throw new InvalidOperationException($"Expected {typeof(TException).Name}.");
    }

    private static void Equal<T>(T expected, T actual, string message)
        where T : notnull
    {
        if (!EqualityComparer<T>.Default.Equals(expected, actual))
        {
            throw new InvalidOperationException($"{message} Expected {expected}, found {actual}.");
        }
    }

    private static void SequenceEqual(byte[] expected, byte[] actual)
    {
        if (!expected.AsSpan().SequenceEqual(actual))
        {
            throw new InvalidOperationException("Equivalent configurations produced different PDFs.");
        }
    }

    private sealed record TestContext(string CustomFontPath, string FontPackPath);
}
