UPDATE recipes SET publication = CASE
    WHEN url ILIKE '%seriouseats.com%' THEN 'Serious Eats'
    WHEN url ILIKE '%cooking.nytimes.com%' THEN 'NYT Cooking'
    WHEN url ILIKE '%bonappetit.com%' THEN 'Bon Appétit'
    WHEN url ILIKE '%allrecipes.com%' THEN 'Allrecipes'
    WHEN url ILIKE '%foodnetwork.com%' THEN 'Food Network'
    WHEN url ILIKE '%simplyrecipes.com%' THEN 'Simply Recipes'
    WHEN url ILIKE '%pinchofyum.com%' THEN 'Pinch of Yum'
    WHEN url ILIKE '%halfbakedharvest.com%' THEN 'Half Baked Harvest'
    WHEN url ILIKE '%budgetbytes.com%' THEN 'Budget Bytes'
    WHEN url ILIKE '%cookieandkate.com%' THEN 'Cookie and Kate'
    WHEN url ILIKE '%food52.com%' THEN 'Food52'
    WHEN url ILIKE '%loveandlemons.com%' THEN 'Love and Lemons'
    WHEN url ILIKE '%minimalistbaker.com%' THEN 'Minimalist Baker'
    WHEN url ILIKE '%recipetineats.com%' THEN 'RecipeTin Eats'
    WHEN url ILIKE '%tasteofhome.com%' THEN 'Taste of Home'
    ELSE INITCAP(REPLACE(REPLACE(
        SPLIT_PART(REGEXP_REPLACE(SPLIT_PART(SPLIT_PART(url, '://', 2), '/', 1), '^(www|m)\.', ''), '.', 1),
    '-', ' '), '_', ' '))
END
WHERE publication IS NULL;
